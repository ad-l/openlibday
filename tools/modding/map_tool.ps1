param(
    [Parameter(Mandatory=$true)]
    [ValidateSet("Export", "ExportObjects", "ExportCatalog", "Apply", "Register")]
    [string]$Action,
    [Parameter(Mandatory=$true)]
    [string]$Source,
    [Parameter(Mandatory=$true)]
    [string]$Output,
    [string]$Csv,
    [string]$GlobalDatabase,
    [string]$TerrainCsv,
    [string]$BuildingCsv,
    [string]$BuildingTypesCsv,
    [string]$SurfaceJson,
    [string]$UnitJson,
    [string]$DeploymentJson,
    [string]$ScenarioJson,
    [string]$ScriptJson,
    [ValidateSet("", "banks")]
    [string]$BridgeTerrain = "",
    [ValidateSet("", "square")]
    [string]$ResizeMap = "",
    [string]$ScenarioDatabase,
    [int]$NextUnitId = -1,
    [int]$NextBuildingId = -1,
    [string]$Member,
    [string]$Title,
    [int]$TemplateCounter = 1
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest
if ([Environment]::Is64BitProcess) {
    throw "Run this script using 32-bit Windows PowerShell (SysWOW64); Jet 4.0 is 32-bit."
}

function Open-Database([string]$Path, [bool]$ReadOnly) {
    $connection = New-Object -ComObject ADODB.Connection
    $builder = New-Object System.Data.OleDb.OleDbConnectionStringBuilder
    $builder.Provider = "Microsoft.Jet.OLEDB.4.0"
    $builder["Data Source"] = (Resolve-Path -LiteralPath $Path).Path
    $builder["Mode"] = $(if ($ReadOnly) { "Read" } else { "Share Exclusive" })
    $connection.Open($builder.ConnectionString)
    return $connection
}

function Query-Rows($Connection, [string]$Sql) {
    $recordset = $Connection.Execute($Sql)
    try {
        while (-not $recordset.EOF) {
            $row = [ordered]@{}
            for ($i = 0; $i -lt $recordset.Fields.Count; $i++) {
                $field = $recordset.Fields.Item($i)
                $row[$field.Name] = $field.Value
            }
            [pscustomobject]$row
            $recordset.MoveNext()
        }
    } finally {
        $recordset.Close()
    }
}

function Check-Grid($Rows) {
    $size = [int][Math]::Sqrt($Rows.Count)
    if ($size * $size -ne $Rows.Count -or $size -lt 1 -or $size -gt 256) {
        throw "Map must be a complete square grid with byte-sized coordinates."
    }
    $seen = @{}
    foreach ($row in $Rows) {
        if ([string]$row.PosX -notmatch '^[0-9]+$' -or [string]$row.PosY -notmatch '^[0-9]+$') {
            throw "Coordinates must be nonempty integer values."
        }
        $x = [int]$row.PosX
        $y = [int]$row.PosY
        $key = "$x,$y"
        if ($x -lt 0 -or $x -ge $size -or $y -lt 0 -or $y -ge $size -or $seen.ContainsKey($key)) {
            throw "Map has out-of-bounds or duplicate coordinates: $key"
        }
        $seen[$key] = $true
    }
    return $size
}

function Get-Objects($Connection) {
    $buildings = @(Query-Rows $Connection "SELECT * FROM [Buildings On Map] ORDER BY [id]")
    $info = @(Query-Rows $Connection "SELECT [NextID_Buildings],[NextID_Units],[TitleEnglish],
        [descriptionEnglish],[turn_qty],[One Off Deployment] FROM [Scenario Info]")
    if ($info.Count -ne 1) { throw "Expected one scenario metadata row." }
    $references = @()
    $unitReferences = @()
    $fixedReferences = @()
    foreach ($table in @("Objectives ID List", "Unit Order ID List", "Unit Order Pos List",
                         "Attack Army", "Counter Attack", "Reinforcement")) {
        foreach ($row in @(Query-Rows $Connection "SELECT * FROM [$table]")) {
            foreach ($field in @("Id", "Unit ID", "UnitID")) {
                if ($row.PSObject.Properties.Name -contains $field) {
                    $references += [int]$row.$field
                    if ($table -ne "Reinforcement") { $unitReferences += [int]$row.$field }
                    if ($table -in @("Attack Army", "Counter Attack")) { $fixedReferences += [int]$row.$field }
                }
            }
        }
    }
    $blocked = @()
    $resizeReservations = @()
    foreach ($table in @("HQ on Map", "Attack Army", "Counter Attack", "Taurans On Map")) {
        foreach ($row in @(Query-Rows $Connection "SELECT * FROM [$table]")) {
            $resizeReservations += [pscustomobject]@{
                table = $table; counter = $row.Counter
                x = $(if ($row.PSObject.Properties.Name -contains "PosX") { [int]$row.PosX } else { 0 })
                y = $(if ($row.PSObject.Properties.Name -contains "PosY") { [int]$row.PosY } else { 0 })
            }
        }
    }
    $reserved = @()
    foreach ($table in @("Units On Map", "Automated Units", "Taurans On Map", "Mines On Map",
                         "Walls On Map", "Roads On Map", "Ruins On Map", "Deployment",
                         "Attack Army", "Counter Attack", "Reinforcement")) {
        foreach ($row in @(Query-Rows $Connection "SELECT * FROM [$table]")) {
            if ($table -in @("Units On Map", "Taurans On Map")) { $reserved += [int]$row.Id }
            if ($row.PSObject.Properties.Name -contains "PosX" -and
                $row.PSObject.Properties.Name -contains "PosY") {
                $blocked += [pscustomobject]@{ x = [int]$row.PosX; y = [int]$row.PosY; reason = $table }
            }
            if ($table -eq "Automated Units") {
                foreach ($prefix in @("Start", "Dest")) {
                    $blocked += [pscustomobject]@{
                        x = [int]$row."${prefix}X"; y = [int]$row."${prefix}Y"; reason = $table
                    }
                }
            }
        }
    }
    return [pscustomobject]@{
        buildings = $buildings; nextBuildingId = [int]$info[0].NextID_Buildings
        references = $references; blocked = $blocked; reservedIds = $reserved
        resizeReservations = $resizeReservations
        units = @(Query-Rows $Connection "SELECT * FROM [Units On Map] ORDER BY [ID]")
        reinforcements = @(Query-Rows $Connection "SELECT * FROM [Reinforcement] ORDER BY [UnitID]")
        unitReferences = $unitReferences; nextUnitId = [int]$info[0].NextID_Units
        fixedReferences = $fixedReferences; scripts = Get-Scripts $Connection
        roads = @(Query-Rows $Connection "SELECT * FROM [Roads On Map] ORDER BY [Counter]")
        walls = @(Query-Rows $Connection "SELECT * FROM [Walls On Map] ORDER BY [Counter]")
        decors = @(Query-Rows $Connection "SELECT * FROM [Ruins On Map] ORDER BY [Counter]")
        mines = @(Query-Rows $Connection "SELECT * FROM [Mines On Map] ORDER BY [Counter]")
        deployments = @(Query-Rows $Connection "SELECT [Counter],[player],[PosX],[PosY],[Turn] FROM [Deployment] ORDER BY [Counter]")
        scenario = [pscustomobject]@{
            title = [string]$info[0].TitleEnglish; briefing = [string]$info[0].descriptionEnglish
            turns = [int]$info[0].turn_qty; points = [int]$info[0]."One Off Deployment"
            available = @(Query-Rows $Connection "SELECT [Enum],[Available] FROM [One Off Units Available] ORDER BY [Enum]" |
                ForEach-Object { [pscustomobject]@{ kind = [int]$_.Enum; enabled = [bool]$_.Available } })
        }
    }
}

function Get-ScriptColumns {
    return [ordered]@{
        "Automated Units" = @("Counter","AutomatedType","StartX","StartY","DestX","DestY","TurnArrival")
        "Objective" = @("Counter","ObjectifId","TurnNumber")
        "Objectives ID List" = @("Counter","Id")
        "Objectives Pos List" = @("Counter","PosX","PosY")
        "Unit Order ID List" = @("Counter","Unit ID","Id")
        "Unit Order Pos List" = @("Counter","Unit ID","PosX","PosY")
    }
}

function Get-Scripts($Connection) {
    $result = [ordered]@{}
    foreach ($table in (Get-ScriptColumns).Keys) {
        $names = ((Get-ScriptColumns)[$table] | ForEach-Object { "[$_]" }) -join ","
        $result[$table] = @(Query-Rows $Connection "SELECT $names FROM [$table]")
    }
    return [pscustomobject]$result
}

function Apply-Scripts($Connection, [int]$Size) {
    $incoming = Get-Content -LiteralPath $ScriptJson -Raw -Encoding UTF8 | ConvertFrom-Json
    $columns = Get-ScriptColumns
    if ($null -eq $incoming -or @($incoming.PSObject.Properties).Count -ne $columns.Count -or
        @($columns.Keys | Where-Object { $incoming.PSObject.Properties.Name -notcontains $_ }).Count) {
        throw "Script JSON must contain exactly the six native script tables."
    }
    $oldTables = Get-Scripts $Connection
    $metadata = Get-Objects $Connection
    $entities = @{}
    $starting = @{}
    $layers = Get-UnitLayers $metadata
    foreach ($row in $layers.units) { $entities[[int]$row.ID] = $true; $starting[[int]$row.ID] = $true }
    $orderKinds = @{}
    foreach ($row in $layers.units) { $orderKinds[[int]$row.ID] = [int]$row.OrderID }
    $oldOrderKinds = @{}
    foreach ($row in $metadata.units) { $oldOrderKinds[[int]$row.ID] = [int]$row.OrderID }
    foreach ($row in $layers.reinforcements) { $entities[[int]$row.UnitID] = $true }
    $buildings = if ($BuildingCsv) { @(Import-Csv -LiteralPath $BuildingCsv) } else { $metadata.buildings }
    foreach ($row in $buildings) { $entities[[int]$row.id] = $true }
    $verifiedTypes = @(0,4,5,6,7,11,12,13,14,19,20,21,22,23,25)
    $changes = @{}
    $deletions = @{}
    foreach ($table in $columns.Keys) {
        if ($incoming.$table -isnot [Array]) { throw "Script table $table must be an array." }
        $old = @{}
        foreach ($row in $oldTables.$table) { $old[[int]$row.Counter] = $row }
        $seen = @{}
        $keys = @{}
        $changed = @{}
        foreach ($row in $incoming.$table) {
            if ($null -eq $row -or @($row.PSObject.Properties).Count -ne $columns[$table].Count -or
                @($columns[$table] | Where-Object { $row.PSObject.Properties.Name -notcontains $_ }).Count) {
                throw "Script $table requires exactly: $($columns[$table] -join ',')."
            }
            foreach ($column in $columns[$table]) {
                if (($row.$column -isnot [int] -and $row.$column -isnot [long]) -or
                    $row.$column -lt [int]::MinValue -or $row.$column -gt [int]::MaxValue) {
                    throw "Script $table $column must be a signed 32-bit integer."
                }
            }
            $counter = [int]$row.Counter
            if ($seen.ContainsKey($counter) -or ($counter -ge 0 -and -not $old.ContainsKey($counter))) {
                throw "Unknown/duplicate script counter in $table`: $counter"
            }
            $seen[$counter] = $true
            $key = ($columns[$table] | Where-Object { $_ -ne "Counter" } |
                ForEach-Object { [string]$row.$_ }) -join ","
            if (-not $keys.ContainsKey($key)) { $keys[$key] = @() }
            $keys[$key] += $counter
            $different = -not $old.ContainsKey($counter)
            if (-not $different) {
                foreach ($column in $columns[$table]) {
                    if ([int]$row.$column -ne [int]$old[$counter].$column) { $different = $true }
                }
            }
            if (-not $different) { continue }
            $changed[$counter] = $row
            $positions = @()
            switch ($table) {
                "Automated Units" {
                    if ($row.AutomatedType -notin $verifiedTypes -or $row.TurnArrival -lt 1 -or $row.TurnArrival -gt 99) {
                        throw "Support edits require a verified type and arrival turn 1..99."
                    }
                    if ($old.ContainsKey($counter) -and ($old[$counter].AutomatedType -notin $verifiedTypes -or
                        $old[$counter].TurnArrival -lt 1 -or $old[$counter].TurnArrival -gt 99)) {
                        throw "Unverified support events must be preserved."
                    }
                    if ($row.AutomatedType -notin @(0,7) -and (
                        $row.StartX -ne $row.DestX -or $row.StartY -ne $row.DestY)) {
                        throw "Non-transport support requires identical start/destination."
                    }
                    $positions = @(@($row.StartX,$row.StartY), @($row.DestX,$row.DestY))
                }
                "Objective" {
                    if ($old.Count -ne 1 -or $incoming.$table.Count -ne 1 -or -not $old.ContainsKey($counter) -or
                        $row.ObjectifId -notin 0..6 -or $old[$counter].ObjectifId -notin 0..6) {
                        throw "Only one verified existing campaign objective can be edited."
                    }
                    if ($row.TurnNumber -ne $old[$counter].TurnNumber -and (
                        $row.ObjectifId -ne 1 -or $row.TurnNumber -lt 0 -or $row.TurnNumber -gt 99)) {
                        throw "Only Hold Position's raw threshold is editable (0..99)."
                    }
                }
                default {
                    if ($table.StartsWith("Unit Order") -and -not $starting.ContainsKey([int]$row."Unit ID")) {
                        throw "AI target rows must belong to an incoming starting unit."
                    }
                    if ($table.EndsWith("ID List")) {
                        if (-not $entities.ContainsKey([int]$row.Id)) { throw "Target ID $($row.Id) is absent." }
                    } else { $positions = @(,@($row.PosX,$row.PosY)) }
                }
            }
            foreach ($position in $positions) {
                if ($position[0] -lt 0 -or $position[1] -lt 0 -or $position[0] -ge $Size -or $position[1] -ge $Size) {
                    throw "Script coordinates must be inside the map."
                }
            }
        }
        if ($table -ne "Automated Units") {
            foreach ($counters in $keys.Values) {
                if ($counters.Count -gt 1 -and @($counters | Where-Object { $changed.ContainsKey($_) }).Count) {
                    throw "New/changed script targets cannot introduce duplicates."
                }
            }
        }
        $removed = @($old.Keys | Where-Object { -not $seen.ContainsKey($_) })
        if ($table -eq "Objective" -and ($removed.Count -or $incoming.$table.Count -ne $old.Count)) {
            throw "Objective records cannot be added or deleted."
        }
        if ($table -eq "Automated Units") {
            foreach ($counter in $removed) {
                if ($old[$counter].AutomatedType -notin $verifiedTypes -or $old[$counter].TurnArrival -lt 1 -or
                    $old[$counter].TurnArrival -gt 99) { throw "Unverified support events must be preserved." }
            }
        }
        if ($table.StartsWith("Objectives ") -and ($changed.Count -or $removed.Count) -and (
            $oldTables.Objective.Count -ne 1 -or $oldTables.Objective[0].ObjectifId -notin 0..6)) {
            throw "Deathmatch/internal objective targets must be preserved."
        }
        $changes[$table] = $changed
        $deletions[$table] = $removed
    }
    foreach ($table in @("Objectives ID List", "Objectives Pos List", "Unit Order ID List", "Unit Order Pos List")) {
        $affected = @($changes[$table].Values)
        foreach ($counter in $deletions[$table]) {
            $affected += @($oldTables.$table | Where-Object { [int]$_.Counter -eq $counter })
        }
        if ($table.StartsWith("Objectives ")) {
            $kinds = if ($table.EndsWith("ID List")) { @(0,2,4,5,6) } else { @(0,1,2,3,4) }
            if ($affected.Count -and $incoming.$table.Count -and $incoming.Objective[0].ObjectifId -notin $kinds) {
                throw "This objective kind does not use the edited target collection."
            }
        } else {
            $kinds = if ($table.EndsWith("ID List")) { @(3,4,6) } else { @(1,2,5) }
            foreach ($id in @($affected | ForEach-Object { [int]$_."Unit ID" } | Select-Object -Unique)) {
                if ($oldOrderKinds.ContainsKey($id) -and $oldOrderKinds[$id] -notin 0..6) {
                    throw "Targets of unverified AI orders must be preserved."
                }
                if (@($incoming.$table | Where-Object { [int]$_."Unit ID" -eq $id }).Count -and (
                    -not $orderKinds.ContainsKey($id) -or $orderKinds[$id] -notin $kinds)) {
                    throw "Choose ID targets for Escort/Defend/Attack or tile targets for Goto/Goto Fast/Patrol."
                }
            }
        }
    }
    foreach ($table in $columns.Keys) {
        foreach ($counter in $deletions[$table]) {
            $null = $Connection.Execute("DELETE FROM [$table] WHERE [Counter]=$counter")
        }
        foreach ($row in $incoming.$table) {
            $counter = [int]$row.Counter
            if (-not $changes[$table].ContainsKey($counter)) { continue }
            $fields = @($columns[$table] | Where-Object { $_ -ne "Counter" })
            if ($counter -ge 0) {
                $assignments = ($fields | ForEach-Object { "[$_]=$($row.$_)" }) -join ","
                $null = $Connection.Execute("UPDATE [$table] SET $assignments WHERE [Counter]=$counter")
            } else {
                $names = ($fields | ForEach-Object { "[$_]" }) -join ","
                $values = ($fields | ForEach-Object { [string]$row.$_ }) -join ","
                $null = $Connection.Execute("INSERT INTO [$table] ($names) VALUES ($values)")
            }
        }
    }
}

function Apply-Deployment($Connection, [int]$Size) {
    $text = Get-Content -LiteralPath $DeploymentJson -Raw -Encoding UTF8
    if (-not $text.Trim().StartsWith("[") -or -not $text.Trim().EndsWith("]")) {
        throw "Deployment JSON must be an array of records."
    }
    $rows = ConvertFrom-Json -InputObject $text
    $columns = @("Counter", "player", "PosX", "PosY", "Turn")
    $old = @{}
    foreach ($row in @(Query-Rows $Connection "SELECT * FROM [Deployment]")) { $old[[int]$row.Counter] = $row }
    $incoming = @{}
    $positions = @{}
    $changed = @{}
    foreach ($row in $rows) {
        if ($null -eq $row -or @($row.PSObject.Properties).Count -ne $columns.Count -or
            @($columns | Where-Object { $row.PSObject.Properties.Name -notcontains $_ }).Count -ne 0) {
            throw "Deployment records require Counter, player, PosX, PosY and Turn."
        }
        foreach ($column in $columns) {
            if (($row.$column -isnot [int] -and $row.$column -isnot [long]) -or
                $row.$column -lt [int]::MinValue -or $row.$column -gt [int]::MaxValue) {
                throw "Deployment $column must be a 32-bit integer."
            }
        }
        $counter = [int]$row.Counter
        if ($incoming.ContainsKey($counter)) { throw "Duplicate deployment counter: $counter" }
        if ($counter -ge 0 -and -not $old.ContainsKey($counter)) { throw "Unknown deployment counter: $counter" }
        $previous = $old[$counter]
        $different = -not $previous
        if ($previous) {
            foreach ($column in $columns) {
                if ([int]$previous.$column -ne [int]$row.$column) { $different = $true }
            }
        }
        if ($different) {
            if ($previous -and ([int]$previous.player -notin 1..4 -or [int]$previous.Turn -ne 0)) {
                throw "AI/unknown-player/nonzero-turn deployment records must be preserved."
            }
            if ([int]$row.player -notin 1..4 -or [int]$row.Turn -ne 0 -or
                [int]$row.PosX -lt 0 -or [int]$row.PosY -lt 0 -or
                [int]$row.PosX -ge $Size -or [int]$row.PosY -ge $Size) {
                throw "Edited deployment cells require player 1..4, Turn 0 and in-bounds coordinates."
            }
            $changed[$counter] = $true
        }
        $incoming[$counter] = $row
        $key = "$($row.PosX),$($row.PosY)"
        if (-not $positions.ContainsKey($key)) { $positions[$key] = @() }
        $positions[$key] += $counter
    }
    foreach ($counter in $changed.Keys) {
        $row = $incoming[$counter]
        if ($positions["$($row.PosX),$($row.PosY)"].Count -gt 1) {
            throw "Edited deployment cells cannot overlap another deployment record."
        }
    }
    foreach ($counter in $old.Keys) {
        if (-not $incoming.ContainsKey($counter)) {
            if ([int]$old[$counter].player -notin 1..4 -or [int]$old[$counter].Turn -ne 0) {
                throw "AI/unknown-player/nonzero-turn deployment records cannot be deleted."
            }
            $null = $Connection.Execute("DELETE FROM [Deployment] WHERE [Counter]=$counter")
        }
    }
    foreach ($counter in $changed.Keys) {
        $row = $incoming[$counter]
        if ($counter -ge 0) {
            $null = $Connection.Execute("UPDATE [Deployment] SET [player]=$($row.player),
                [PosX]=$($row.PosX),[PosY]=$($row.PosY),[Turn]=0 WHERE [Counter]=$counter")
        } else {
            $null = $Connection.Execute("INSERT INTO [Deployment] ([player],[PosX],[PosY],[Turn])
                VALUES ($($row.player),$($row.PosX),$($row.PosY),0)")
        }
    }
}

function Set-ScenarioField($Connection, [string]$Field, $Value, [int]$Type, [string]$Where = "") {
    $command = New-Object -ComObject ADODB.Command
    $command.ActiveConnection = $Connection
    $command.CommandText = "UPDATE [Scenario Info] SET [$Field]=? $Where"
    $parameter = $command.CreateParameter("value", $Type, 1, [Math]::Max(1, ([string]$Value).Length), $Value)
    $command.Parameters.Append($parameter)
    $null = $command.Execute()
}

function Apply-Scenario($Connection) {
    $settings = Get-Content -LiteralPath $ScenarioJson -Raw -Encoding UTF8 | ConvertFrom-Json
    $fields = @("title", "briefing", "turns", "points", "available")
    if ($null -eq $settings -or @($settings.PSObject.Properties).Count -ne $fields.Count -or
        @($fields | Where-Object { $settings.PSObject.Properties.Name -notcontains $_ }).Count -ne 0) {
        throw "Scenario JSON requires title, briefing, turns, points and available."
    }
    if ($settings.title -isnot [string] -or -not $settings.title.Trim() -or $settings.title.Length -gt 50 -or
        $settings.briefing -isnot [string] -or $settings.title.Contains([char]0) -or
        $settings.briefing.Contains([char]0)) { throw "Scenario name must be 1..50 characters; text cannot contain NUL." }
    foreach ($field in @("turns", "points")) {
        if (($settings.$field -isnot [int] -and $settings.$field -isnot [long]) -or
            $settings.$field -lt $(if ($field -eq "turns") { 1 } else { 0 }) -or
            $settings.$field -gt $(if ($field -eq "turns") { 100 } else { 32767 })) {
            throw "Scenario turns must be 1..100 and points 0..32767, as integers."
        }
    }
    if ($settings.available -isnot [Array]) { throw "Available units must be an array." }
    $types = Get-UnitTypes
    $incoming = @{}
    foreach ($row in $settings.available) {
        if ($null -eq $row -or @($row.PSObject.Properties).Count -ne 2 -or
            $row.PSObject.Properties.Name -notcontains "kind" -or
            $row.PSObject.Properties.Name -notcontains "enabled") { throw "Invalid available-unit row." }
        if (($row.kind -isnot [int] -and $row.kind -isnot [long]) -or $row.enabled -isnot [bool] -or
            -not $types.ContainsKey("$($row.kind),0") -or
            ($row.kind -ge 35 -and $row.kind -ne 49 -and $row.kind -notin (54..59)) -or
            $incoming.ContainsKey([int]$row.kind)) { throw "Invalid or duplicate skirmish unit type." }
        $incoming[[int]$row.kind] = $row.enabled
    }
    $old = @{}
    foreach ($row in @(Query-Rows $Connection "SELECT [Counter],[Enum],[Available] FROM [One Off Units Available]")) {
        $id = [int]$row.Enum
        if ($old.ContainsKey($id) -or -not $incoming.ContainsKey($id)) {
            throw "Existing roster rows must be unique and retained; disable rather than delete."
        }
        $old[$id] = $row
    }
    $info = @(Query-Rows $Connection "SELECT [TitleEnglish],[descriptionEnglish],[turn_qty],[One Off Deployment] FROM [Scenario Info]")
    if ($info.Count -ne 1) { throw "Expected one scenario metadata row." }
    foreach ($pair in @(@("title", "TitleEnglish", 202), @("briefing", "descriptionEnglish", 203),
                         @("turns", "turn_qty", 3), @("points", "One Off Deployment", 3))) {
        $value = $settings.($pair[0])
        if ([string]$value -cne [string]$info[0].($pair[1])) {
            Set-ScenarioField $Connection $pair[1] $value $pair[2]
        }
    }
    foreach ($id in $incoming.Keys) {
        $enabled = $(if ($incoming[$id]) { -1 } else { 0 })
        if ($old.ContainsKey($id)) {
            if ([bool]$old[$id].Available -ne $incoming[$id]) {
                $null = $Connection.Execute("UPDATE [One Off Units Available] SET [Available]=$enabled WHERE [Counter]=$([int]$old[$id].Counter)")
            }
        } else {
            $null = $Connection.Execute("INSERT INTO [One Off Units Available] ([Enum],[Available]) VALUES ($id,$enabled)")
        }
    }
}

function Get-UnitLayers($Metadata) {
    if (-not $UnitJson) {
        return [pscustomobject]@{ units = $Metadata.units; reinforcements = $Metadata.reinforcements }
    }
    $inputUnits = Get-Content -LiteralPath $UnitJson -Raw | ConvertFrom-Json
    if ($null -eq $inputUnits -or @($inputUnits.PSObject.Properties).Count -ne 2 -or
        $inputUnits.PSObject.Properties.Name -notcontains "units" -or
        $inputUnits.PSObject.Properties.Name -notcontains "reinforcements" -or
        $inputUnits.units -isnot [Array] -or $inputUnits.reinforcements -isnot [Array]) {
        throw "Unit JSON requires units and reinforcements arrays, even when empty."
    }
    return $inputUnits
}

function Get-UnitTypes {
    $types = @{}
    foreach ($row in @(Import-Csv -LiteralPath (Join-Path $PSScriptRoot "unit-types.csv"))) {
        $types["$($row.typeID),$($row.unitTechLevel)"] = $row
    }
    return $types
}

function Get-UnitBlocks($Metadata) {
    $layers = Get-UnitLayers $Metadata
    $types = Get-UnitTypes
    foreach ($layer in @("units", "reinforcements")) {
        foreach ($row in $layers.$layer) {
            $key = if ($layer -eq "units") { "$($row.unitType),$($row.unitTL)" } else { "$($row.type),0" }
            if (-not $types.ContainsKey($key)) { throw "Unknown unit type/technology: $key" }
            Building-Footprint $row $types[$key]
        }
    }
}

function Unit-TerrainCompatible($Type, $Terrain) {
    $movement = [int]$Type.Movement
    $kind = [int]$Terrain.type
    $clear = [int]$Terrain.Overlapping -eq 0
    if ($movement -in @(4, 6, 8)) { return $kind -eq 1 -and $clear }
    if ($movement -eq 5) { return $true }
    if ($movement -in @(3, 9)) { return $kind -in @(1, 2, 3) -and $clear }
    return ($kind -eq 3 -and $clear) -or ($movement -eq 2 -and $kind -eq 5)
}

function Apply-Units($Connection, $MapRows, $TerrainTypes) {
    if (-not $TerrainCsv) { throw "Unit edits require -TerrainCsv." }
    $metadata = Get-Objects $Connection
    $layers = Get-UnitLayers $metadata
    $types = Get-UnitTypes
    $tables = @{ units = "Units On Map"; reinforcements = "Reinforcement" }
    $columns = @{
        units = @("Counter","ID","unitType","unitTL","playerOwner","facing","currentAR","currentAP",
                  "negAP","PosX","PosY","-------------","OrderID","Turn","Probability","EnemyUnitType")
        reinforcements = @("Counter","UnitID","type","posX","posY","turn","order")
    }
    $old = @{}
    foreach ($layer in @("units", "reinforcements")) {
        foreach ($row in $metadata.$layer) {
            $id = [int]$(if ($layer -eq "units") { $row.ID } else { $row.UnitID })
            if ($old.ContainsKey($id)) { throw "Duplicate unit ID in source: $id" }
            $old[$id] = @{ layer = $layer; row = $row }
        }
    }
    $incoming = @{}
    $map = @{}
    $terrain = @{}
    foreach ($row in $MapRows) { $map["$($row.PosX),$($row.PosY)"] = $row }
    foreach ($row in $TerrainTypes) { $terrain[[string]$row.id] = $row }
    $blocked = @{}
    foreach ($row in $metadata.blocked) {
        if ($row.reason -notin @("Units On Map", "Reinforcement", "Roads On Map", "Walls On Map", "Ruins On Map") -and
            (-not $ScriptJson -or $row.reason -ne "Automated Units")) {
            $blocked["$($row.x),$($row.y)"] = $row.reason
        }
    }
    foreach ($row in $metadata.walls) { $blocked["$($row.posX),$($row.posY)"] = "wall" }
    $decorTypes = @{}
    foreach ($row in @(Import-Csv -LiteralPath (Join-Path $PSScriptRoot "decor-types.csv"))) {
        $decorTypes[[string]$row.id] = $row
    }
    foreach ($row in $metadata.decors) {
        if ([int]$decorTypes[[string]$row.id].can_move_through -eq 0) {
            $blocked["$($row.PosX),$($row.PosY)"] = "blocking decor"
        }
    }
    $buildingTypes = @{}
    foreach ($row in @(Import-Csv -LiteralPath (Join-Path $PSScriptRoot "building-types.csv"))) {
        $buildingTypes[[int]$row.TypeID] = $row
    }
    $buildings = if ($BuildingCsv) { @(Import-Csv -LiteralPath $BuildingCsv) } else { $metadata.buildings }
    foreach ($row in $buildings) {
        foreach ($key in @(Building-Footprint $row $buildingTypes[[int]$row.buildingType])) {
            $blocked[$key] = "building"
        }
    }
    foreach ($layer in @("units", "reinforcements")) {
        foreach ($row in $layers.$layer) {
            if ($null -eq $row -or @($row.PSObject.Properties).Count -ne $columns[$layer].Count -or
                @($columns[$layer] | Where-Object { $row.PSObject.Properties.Name -notcontains $_ }).Count) {
                throw "$layer rows require exactly: $($columns[$layer] -join ',')."
            }
            $flags = if ($layer -eq "units") { @("-------------") } else { @("order") }
            foreach ($column in $columns[$layer] | Where-Object { $_ -ne "Counter" }) {
                if ($column -in $flags) {
                    if ([string]$row.$column -notin @("0","1")) { throw "$column must be 0 or 1." }
                } else {
                    if ([string]$row.$column -notmatch '^[0-9]+$') { throw "Unit $column must be a nonnegative integer." }
                    $null = [int]$row.$column
                }
            }
            $id = [int]$(if ($layer -eq "units") { $row.ID } else { $row.UnitID })
            $typeKey = if ($layer -eq "units") { "$($row.unitType),$($row.unitTL)" } else { "$($row.type),0" }
            if ($id -ge 1000000 -or $incoming.ContainsKey($id) -or -not $types.ContainsKey($typeKey)) {
                throw "Invalid/duplicate unit ID or unknown type/technology: $id"
            }
            $previous = $old[$id]
            $sameLayer = $previous -and $previous.layer -eq $layer
            if ($sameLayer) {
                if ([string]$row.Counter -notmatch '^[0-9]+$' -or
                    [int]$row.Counter -ne [int]$previous.row.Counter) {
                    throw "Existing unit IDs and database counters must remain paired."
                }
            } elseif ($row.Counter -ne "") { throw "New or converted units require an empty AutoNumber counter." }
            $allocationReferences = if ($ScriptJson) { $script:originalScriptReferences } else { $metadata.references }
            if (-not $previous -and ($id -lt $metadata.nextUnitId -or $metadata.reservedIds -contains $id -or
                                    $allocationReferences -contains $id)) {
                throw "New units require a fresh game ID."
            }
            if ($previous -and -not $sameLayer -and $metadata.unitReferences -contains $id) {
                throw "Unit $id is referenced by mission objectives/orders; arrival category changes are blocked."
            }
            if ($previous -and $previous.layer -eq "units" -and $layer -eq "reinforcements" -and
                [int]$previous.row.playerOwner -ne 0) {
                throw "Native reinforcements belong to owner 0 (AI); player units must start on the map."
            }
            $edited = -not $sameLayer
            if ($sameLayer) {
                foreach ($column in $columns[$layer] | Where-Object { $_ -ne "Counter" }) {
                    $different = if ($column -in $flags) {
                        [bool][int]$row.$column -ne [bool]$previous.row.$column
                    } else { [int]$row.$column -ne [int]$previous.row.$column }
                    if ($different) { $edited = $true }
                }
            }
            $definition = $types[$typeKey]
            $arrival = if ($layer -eq "units") { 0 } else { [int]$row.turn }
            if ($layer -eq "reinforcements" -and $arrival -gt 100) { throw "Reinforcement turn must be 0..100." }
            if ($layer -eq "units" -and $edited) {
                if ([int]$row.playerOwner -gt 4 -or [int]$row.facing -gt 7 -or
                    [int]$row.currentAR -lt 1 -or [int]$row.currentAR -gt [int]$definition.Armor -or
                    [int]$row.currentAP -gt [int]$definition.actionPoints) { throw "Invalid unit owner, facing, health or AP." }
                foreach ($column in @("negAP","-------------","OrderID","Turn","Probability","EnemyUnitType")) {
                    $expected = if ($sameLayer) { [int]$previous.row.$column } else {
                        switch ($column) { "-------------" { 1 }; "Probability" { 100 }; "EnemyUnitType" { 65535 }; default { 0 } }
                    }
                    if ([int]$row.$column -ne $expected -and (
                        -not $ScriptJson -or $column -in @("negAP","-------------"))) {
                        throw "Unit AI/order field $column must be preserved without a script transaction."
                    }
                }
                $orderChanged = -not $sameLayer
                if ($sameLayer) {
                    foreach ($column in @("OrderID","Turn","Probability","EnemyUnitType")) {
                        if ([int]$row.$column -ne [int]$previous.row.$column) { $orderChanged = $true }
                    }
                }
                if ($orderChanged -and $ScriptJson) {
                    if ([int]$row.OrderID -notin 0..6 -or [int]$row.Probability -gt 100 -or
                        ([int]$row.EnemyUnitType -ne 65535 -and -not $types.ContainsKey("$($row.EnemyUnitType),0")) -or
                        ($sameLayer -and [int]$previous.row.OrderID -notin 0..6)) {
                        throw "AI edits require a verified order, retention 0..100 and a known preferred enemy type."
                    }
                }
            }
            $relocating = -not $sameLayer
            if ($sameLayer) {
                $kindColumn = if ($layer -eq "units") { "unitType" } else { "type" }
                $relocating = ([int]$row.$kindColumn -ne [int]$previous.row.$kindColumn -or
                               [int]$row.PosX -ne [int]$previous.row.PosX -or [int]$row.PosY -ne [int]$previous.row.PosY -or
                               ($layer -eq "reinforcements" -and [int]$row.turn -ne [int]$previous.row.turn))
            }
            $incoming[$id] = @{ layer=$layer; row=$row; definition=$definition; arrival=$arrival;
                               edited=$edited; relocating=$relocating; cells=@(Building-Footprint $row $definition) }
        }
    }
    foreach ($id in $incoming.Keys) {
        $item = $incoming[$id]
        if (-not $item.relocating) { continue }
        foreach ($key in $item.cells) {
            if (-not $map.ContainsKey($key)) { throw "Unit $id extends outside the map." }
            if (-not (Unit-TerrainCompatible $item.definition $terrain[$map[$key].TerrainImg])) {
                throw "Unit $id requires compatible terrain at $key."
            }
            if ($blocked.ContainsKey($key)) { throw "Unit $id overlaps $($blocked[$key]) at $key." }
            foreach ($other in $incoming.Keys) {
                if ($other -ne $id -and $incoming[$other].arrival -eq $item.arrival -and
                    $incoming[$other].cells -contains $key) { throw "Units overlap on arrival turn $($item.arrival) at $key." }
            }
        }
    }
    foreach ($id in $old.Keys) {
        if (-not $incoming.ContainsKey($id) -and $metadata.unitReferences -contains $id) {
            throw "Unit $id is referenced by mission objectives/orders; deletion is blocked."
        }
    }
    $minimumNext = [Math]::Max(0, $metadata.nextUnitId)
    foreach ($id in $incoming.Keys) { $minimumNext = [Math]::Max($minimumNext, $id + 1) }
    if ($NextUnitId -lt $minimumNext -or $NextUnitId -gt 1000000) { throw "NextUnitId must exceed unit IDs and preserve the source counter." }
    foreach ($id in $old.Keys) {
        if (-not $incoming.ContainsKey($id) -or $incoming[$id].layer -ne $old[$id].layer) {
            $column = if ($old[$id].layer -eq "units") { "ID" } else { "UnitID" }
            $null = $Connection.Execute("DELETE FROM [$($tables[$old[$id].layer])] WHERE [$column]=$id")
        }
    }
    foreach ($id in $incoming.Keys) {
        $item = $incoming[$id]
        if (-not $item.edited) { continue }
        $row = $item.row
        $fields = @($columns[$item.layer] | Where-Object { $_ -ne "Counter" })
        if ($row.Counter -ne "") {
            $assignments = foreach ($column in $fields) { "[$column]=$([int]$row.$column)" }
            $null = $Connection.Execute("UPDATE [$($tables[$item.layer])] SET $($assignments -join ',') WHERE [Counter]=$([int]$row.Counter)")
        } else {
            $names = ($fields | ForEach-Object { "[$_]" }) -join ","
            $values = ($fields | ForEach-Object { [string][int]$row.$_ }) -join ","
            $null = $Connection.Execute("INSERT INTO [$($tables[$item.layer])] ($names) VALUES ($values)")
        }
    }
    if ($NextUnitId -ne $metadata.nextUnitId -and
        (-not $ScriptJson -or $NextUnitId -ne $script:originalUnitFloor)) {
        $null = $Connection.Execute("UPDATE [Scenario Info] SET [NextID_Units]=$NextUnitId")
    }
}

function Apply-Surfaces($Connection, $MapRows, $TerrainTypes, $OldMap) {
    if (-not $TerrainCsv) { throw "Surface edits require -TerrainCsv." }
    $inputLayers = Get-Content -LiteralPath $SurfaceJson -Raw | ConvertFrom-Json
    $layers = @("roads", "walls", "decors")
    if ($null -ne $inputLayers -and $inputLayers.PSObject.Properties.Name -contains "mines") { $layers += "mines" }
    if ($null -eq $inputLayers -or $inputLayers.PSObject.Properties.Name.Count -ne $layers.Count -or
        @($layers | Where-Object { $inputLayers.PSObject.Properties.Name -notcontains $_ }).Count) {
        throw "Surface JSON requires roads, walls and decors arrays, even when empty."
    }
    $tables = @{ roads = "Roads On Map"; walls = "Walls On Map"; decors = "Ruins On Map" }
    if ($layers -contains "mines") { $tables.mines = "Mines On Map" }
    $mineFlags = @("armed", "hidden_AI", "hidden_1", "hidden_2", "hidden_3", "hidden_4")
    $columns = @{
        roads = @("Counter", "roadtype", "playerOwner", "PosX", "PosY")
        walls = @("Counter", "roadtype", "playerOwner", "currentAR", "posX", "posY", "isDamaged")
        decors = @("Counter", "id", "PosX", "PosY")
        mines = @("Counter", "typeID", "playerOwner", "PosX", "PosY") + $mineFlags
    }
    $types = @{ roads = @{}; walls = @{}; decors = @{}; mines = @{} }
    foreach ($row in @(Import-Csv -LiteralPath (Join-Path $PSScriptRoot "road-types.csv"))) {
        $types.roads[[string]$row.RoadType] = $row
    }
    foreach ($row in @(Import-Csv -LiteralPath (Join-Path $PSScriptRoot "wall-types.csv"))) {
        $types.walls[[string]$row.WallType] = $row
    }
    foreach ($row in @(Import-Csv -LiteralPath (Join-Path $PSScriptRoot "decor-types.csv"))) {
        $types.decors[[string]$row.id] = $row
    }
    foreach ($row in @(Import-Csv -LiteralPath (Join-Path $PSScriptRoot "mine-types.csv"))) {
        if ($row.typeID -notin @("0", "1") -or $row.ImageID -notin @("MINE", "GMINE")) {
            throw "Invalid mine definition."
        }
        $types.mines[[string]$row.typeID] = $row
    }
    $map = @{}
    $terrain = @{}
    foreach ($row in $MapRows) { $map["$($row.PosX),$($row.PosY)"] = $row }
    foreach ($row in $TerrainTypes) { $terrain[[string]$row.id] = $row }
    $metadata = Get-Objects $Connection
    $blocked = @{}
    foreach ($row in $metadata.blocked) {
        if ($row.reason -notin $tables.Values -and $row.reason -notin @("Units On Map", "Reinforcement") -and
            (-not $ScriptJson -or $row.reason -ne "Automated Units")) {
            $blocked["$($row.x),$($row.y)"] = $row.reason
        }
    }
    foreach ($key in @(Get-UnitBlocks $metadata)) { $blocked[$key] = "unit / reinforcement" }
    $buildingTypes = @{}
    foreach ($row in @(Import-Csv -LiteralPath (Join-Path $PSScriptRoot "building-types.csv"))) {
        $buildingTypes[[int]$row.TypeID] = $row
    }
    $buildings = if ($BuildingCsv) { @(Import-Csv -LiteralPath $BuildingCsv) } else { $metadata.buildings }
    $buildingCells = @{}
    foreach ($building in $buildings) {
        if (-not $buildingTypes.ContainsKey([int]$building.buildingType)) { throw "Unknown building type." }
        foreach ($key in @(Building-Footprint $building $buildingTypes[[int]$building.buildingType])) {
            $buildingCells[$key] = $true
        }
    }
    $original = @{}
    $incoming = @{}
    $at = @{}
    $affectedRoads = @{}
    foreach ($row in $MapRows) {
        if (-not $OldMap.ContainsKey([int]$row.Id) -or $row.TerrainImg -ne $OldMap[[int]$row.Id].TerrainImg) {
            $affectedRoads["$($row.PosX),$($row.PosY)"] = $true
        }
    }
    foreach ($layer in $layers) {
        if ($inputLayers.$layer -isnot [Array]) { throw "$layer must be an array, not null or an object." }
        $original[$layer] = @{}
        foreach ($row in $metadata.$layer) { $original[$layer][[int]$row.Counter] = $row }
        $incoming[$layer] = @()
        $seen = @{}
        foreach ($row in $inputLayers.$layer) {
            if ($null -eq $row -or @($row.PSObject.Properties).Count -ne $columns[$layer].Count -or
                @($columns[$layer] | Where-Object { $row.PSObject.Properties.Name -notcontains $_ }).Count) {
                throw "$layer rows require exactly: $($columns[$layer] -join ',')."
            }
            $numeric = @($columns[$layer] | Where-Object { $_ -notin @("Counter", "id", "isDamaged") })
            foreach ($column in $numeric) {
                if ([string]$row.$column -notmatch '^[0-9]+$') { throw "$layer $column must be a nonnegative integer." }
                $null = [int]$row.$column
            }
            $kind = [string]$(if ($layer -eq "mines") { $row.typeID } elseif ($layer -eq "decors") { $row.id } else { $row.roadtype })
            if (-not $types[$layer].ContainsKey($kind) -or $kind -notmatch '^[A-Za-z0-9_]{1,8}$') {
                throw "Unknown or invalid $layer type: $kind"
            }
            if ($layer -eq "walls" -and [string]$row.isDamaged -notin @("0", "1")) {
                throw "Wall isDamaged must be 0 or 1."
            }
            if ($layer -eq "mines") {
                foreach ($flag in $mineFlags) {
                    if ([string]$row.$flag -notin @("0", "1")) { throw "Mine $flag must be 0 or 1." }
                }
            }
            $previous = $null
            if ($row.Counter -ne "") {
                if ([string]$row.Counter -notmatch '^[0-9]+$') { throw "Invalid $layer counter." }
                $counter = [int]$row.Counter
                if (-not $original[$layer].ContainsKey($counter) -or $seen.ContainsKey($counter)) {
                    throw "Existing $layer counters must be unique and present in the source."
                }
                $seen[$counter] = $true
                $previous = $original[$layer][$counter]
            }
            $edited = $null -eq $previous
            if ($previous) {
                foreach ($column in $columns[$layer] | Where-Object { $_ -ne "Counter" }) {
                    $different = if ($column -eq "isDamaged" -or $column -in $mineFlags) {
                        [bool][int]$row.$column -ne [bool]$previous.$column
                    } elseif ($column -eq "id") { $row.$column -cne $previous.$column }
                    else { [int]$row.$column -ne [int]$previous.$column }
                    if ($different) { $edited = $true }
                }
            }
            $relocating = ($null -eq $previous -or [int]$row.PosX -ne [int]$previous.PosX -or
                           [int]$row.PosY -ne [int]$previous.PosY -or
                           $kind -ne [string]$(if ($layer -eq "mines") { $previous.typeID } elseif ($layer -eq "decors") { $previous.id } else { $previous.roadtype }))
            if ($edited) {
                if ($layer -ne "decors" -and [int]$row.playerOwner -gt 4) { throw "$layer owner must be 0..4." }
                if ($layer -eq "walls") {
                    if ([int]$row.currentAR -lt 1 -or [int]$row.currentAR -gt [int]$types.walls[$kind].Armor) {
                        throw "Invalid wall armor."
                    }
                    if ([int]$row.isDamaged -eq 1 -and $kind -ne "0") {
                        throw "Broken-section shapes are available only for concrete walls."
                    }
                }
            }
            $key = "$([int]$row.PosX),$([int]$row.PosY)"
            if ($relocating) {
                if (-not $map.ContainsKey($key)) { throw "$layer object is outside the map." }
                $tile = $terrain[$map[$key].TerrainImg]
                $compatible = ([int]$tile.type -eq 3 -and [int]$tile.Overlapping -eq 0) -or
                              ($layer -eq "roads" -and $kind -eq "1" -and
                               [int]$tile.type -in @(1, 2) -and [int]$tile.Overlapping -eq 0)
                if (-not $compatible) { throw "$layer object requires compatible clear terrain at $key." }
                if ($layer -ne "roads" -and $blocked.ContainsKey($key)) {
                    throw "$layer object overlaps $($blocked[$key]) at $key."
                }
                if ($buildingCells.ContainsKey($key)) { throw "$layer object overlaps a building at $key." }
            }
            $entry = [pscustomobject]@{ row = $row; edited = $edited; relocating = $relocating; key = $key; layer = $layer }
            $incoming[$layer] += $entry
            if (-not $at.ContainsKey($key)) { $at[$key] = @() }
            $at[$key] += $entry
            if ($layer -eq "roads" -and $edited) {
                $affectedRoads[$key] = $true
                if ($previous) { $affectedRoads["$($previous.PosX),$($previous.PosY)"] = $true }
            }
        }
        if ($layer -eq "roads") {
            foreach ($counter in $original[$layer].Keys) {
                if (-not $seen.ContainsKey($counter)) {
                    $row = $original[$layer][$counter]
                    $affectedRoads["$($row.PosX),$($row.PosY)"] = $true
                }
            }
        }
    }
    foreach ($layer in $layers) {
        foreach ($entry in $incoming[$layer]) {
            $overlaps = @($at[$entry.key] | Where-Object {
                $_ -ne $entry -and -not (
                    ($layer -eq "roads" -and $_.layer -eq "mines") -or
                    ($layer -eq "mines" -and $_.layer -eq "roads"))
            })
            if ($entry.relocating -and $overlaps.Count -gt 0) {
                throw "$layer object overlaps another road, wall, decor or mine at $($entry.key)."
            }
        }
    }
    $roads = @{}
    foreach ($entry in $incoming.roads) { $roads[$entry.key] = $entry.row }
    $connectionSprites = @(0, 9, 10, 7, 8, 13, 12, 2, 11, 15, 14, 5, 6, 3, 4, 1)
    $neighbors = @(@(-1, 0, 8), @(1, 0, 4), @(0, -1, 2), @(0, 1, 1))
    $checkRoads = @{}
    foreach ($key in $affectedRoads.Keys) {
        $x, $y = @($key -split ',' | ForEach-Object { [int]$_ })
        $checkRoads[$key] = $true
        foreach ($offset in $neighbors) { $checkRoads["$($x + $offset[0]),$($y + $offset[1])"] = $true }
    }
    foreach ($key in $checkRoads.Keys) {
        if (-not $roads.ContainsKey($key) -or -not $map.ContainsKey($key)) { continue }
        $road = $roads[$key]
        $x, $y = [int]$road.PosX, [int]$road.PosY
        $mask = 0
        foreach ($offset in $neighbors) {
            $neighborKey = "$($x + $offset[0]),$($y + $offset[1])"
            if ($roads.ContainsKey($neighborKey) -and $roads[$neighborKey].roadtype -eq $road.roadtype) {
                $mask += $offset[2]
            }
        }
        $shape = $connectionSprites[$mask]
        $tileKind = [int]$terrain[$map[$key].TerrainImg].type
        if ($tileKind -eq 1 -and $shape -notin @(0, 6, 7, 8, 9, 10, 11)) {
            throw "This road connection has no bridge artwork; bridges must run straight."
        }
        if ($tileKind -eq 2) {
            $adjacent = foreach ($offset in $neighbors) {
                $neighborKey = "$($x + $offset[0]),$($y + $offset[1])"
                if ($map.ContainsKey($neighborKey)) { [int]$terrain[$map[$neighborKey].TerrainImg].type } else { -1 }
            }
            $orientation = if ($adjacent[0] -in @(1, 2) -and $adjacent[1] -eq 3) { 0 }
                           elseif ($adjacent[1] -in @(1, 2) -and $adjacent[0] -eq 3) { 1 }
                           elseif ($adjacent[2] -in @(1, 2) -and $adjacent[3] -eq 3) { 2 } else { 3 }
            if (($orientation -lt 2 -and $shape -notin @(0, 6, 8, 11)) -or
                ($orientation -ge 2 -and $shape -notin @(0, 7, 9, 10))) {
                throw "This road connection has no riverbank artwork; bridges must run straight."
            }
        }
    }
    foreach ($layer in $layers) {
        $retained = @{}
        foreach ($entry in $incoming[$layer]) {
            if ($entry.row.Counter -ne "") { $retained[[int]$entry.row.Counter] = $true }
        }
        foreach ($counter in $original[$layer].Keys) {
            if (-not $retained.ContainsKey($counter)) {
                $null = $Connection.Execute("DELETE FROM [$($tables[$layer])] WHERE [Counter]=$counter")
            }
        }
        foreach ($entry in $incoming[$layer]) {
            if (-not $entry.edited) { continue }
            $row = $entry.row
            $fields = @($columns[$layer] | Where-Object { $_ -ne "Counter" })
            $values = foreach ($column in $fields) {
                if ($column -eq "id") { "'$($row.id)'" } else { [string][int]$row.$column }
            }
            if ($row.Counter -ne "") {
                $assignments = for ($i = 0; $i -lt $fields.Count; $i++) { "[$($fields[$i])]=$($values[$i])" }
                $null = $Connection.Execute("UPDATE [$($tables[$layer])] SET $($assignments -join ',') WHERE [Counter]=$([int]$row.Counter)")
            } else {
                $names = ($fields | ForEach-Object { "[$_]" }) -join ","
                $null = $Connection.Execute("INSERT INTO [$($tables[$layer])] ($names) VALUES ($($values -join ','))")
            }
        }
    }
}

function Building-Footprint($Row, $Type) {
    for ($dx = 0; $dx -lt [int]$Type.SizeX; $dx++) {
        for ($dy = 0; $dy -lt [int]$Type.SizeY; $dy++) {
            "$([int]$Row.PosX + $dx),$([int]$Row.PosY + $dy)"
        }
    }
}

function Apply-Buildings($Connection, $MapRows, $TerrainTypes, $OldMap) {
    if (-not $BuildingTypesCsv -or -not $TerrainCsv) {
        throw "Building edits require -BuildingTypesCsv and -TerrainCsv."
    }
    $types = @{}
    foreach ($type in @(Import-Csv -LiteralPath $BuildingTypesCsv)) { $types[[int]$type.TypeID] = $type }
    $metadata = Get-Objects $Connection
    $old = @{}
    foreach ($row in $metadata.buildings) { $old[[int]$row.id] = $row }
    $incoming = @{}
    $footprints = @{}
    $relocating = @{}
    $edited = @{}
    $map = @{}
    $terrain = @{}
    $blocked = @{}
    foreach ($row in $MapRows) { $map["$($row.PosX),$($row.PosY)"] = $row }
    foreach ($row in $TerrainTypes) { $terrain[[string]$row.id] = $row }
    foreach ($row in $metadata.blocked) {
        if ($row.reason -notin @("Roads On Map", "Walls On Map", "Ruins On Map", "Units On Map", "Reinforcement") -and
            (-not $ScriptJson -or $row.reason -ne "Automated Units")) {
            $blocked["$($row.x),$($row.y)"] = $row.reason
        }
        foreach ($key in @(Get-UnitBlocks $metadata)) { $blocked[$key] = "unit / reinforcement" }
    }
    foreach ($layer in @("roads", "walls", "decors")) {
        foreach ($row in $metadata.$layer) { $blocked["$($row.PosX),$($row.PosY)"] = $layer }
    }
    $columns = @("Counter", "id", "buildingType", "playerOwner", "currentAR", "PosX", "PosY", "IsAiTarget")
    $header = Get-Content -LiteralPath $BuildingCsv -TotalCount 1
    if (-not $header) { throw "Building CSV requires its full header, even when deleting all buildings." }
    $headerColumns = @($header -split ',' | ForEach-Object { $_.Trim('"') })
    if ($headerColumns.Count -ne $columns.Count -or
        @($columns | Where-Object { $headerColumns -notcontains $_ }).Count -ne 0) {
        throw "Building CSV requires Counter,id,buildingType,playerOwner,currentAR,PosX,PosY,IsAiTarget."
    }
    foreach ($row in @(Import-Csv -LiteralPath $BuildingCsv)) {
        foreach ($column in $columns) {
            if ($row.PSObject.Properties.Name -notcontains $column) { throw "Building CSV is missing $column." }
        }
        foreach ($column in @("id", "buildingType", "playerOwner", "currentAR", "PosX", "PosY")) {
            if ($row.$column -notmatch '^[0-9]+$') { throw "Building $column must be a nonnegative integer." }
            $null = [int]$row.$column
        }
        if ($row.IsAiTarget -notin @("0", "1")) { throw "IsAiTarget must be 0 or 1." }
        $id = [int]$row.id
        if ($id -lt 1000000 -or $id -ge [int]::MaxValue -or $incoming.ContainsKey($id) -or
            -not $types.ContainsKey([int]$row.buildingType)) {
            throw "Invalid/duplicate building ID or unknown building type: $id"
        }
        if ($old.ContainsKey($id)) {
            if ($row.Counter -notmatch '^[0-9]+$' -or [int]$row.Counter -ne [int]$old[$id].Counter) {
                throw "Existing building IDs and database counters must remain paired."
            }
            $isEdited = $false
            foreach ($column in @("buildingType", "playerOwner", "currentAR", "PosX", "PosY")) {
                if ([int]$row.$column -ne [int]$old[$id].$column) { $isEdited = $true }
            }
            if ([bool][int]$row.IsAiTarget -ne [bool]$old[$id].IsAiTarget) { $isEdited = $true }
            $isRelocating = ([int]$row.buildingType -ne [int]$old[$id].buildingType -or
                             [int]$row.PosX -ne [int]$old[$id].PosX -or [int]$row.PosY -ne [int]$old[$id].PosY)
        } else {
            $allocationReferences = if ($ScriptJson) { $script:originalScriptReferences } else { $metadata.references }
            if ($row.Counter -ne "" -or $id -lt $metadata.nextBuildingId -or
                $metadata.reservedIds -contains $id -or $allocationReferences -contains $id) {
                throw "New buildings require a fresh game ID and an empty AutoNumber counter."
            }
            $isEdited = $true
            $isRelocating = $true
        }
        $type = $types[[int]$row.buildingType]
        if ($isEdited -and ([int]$row.playerOwner -gt 4 -or [int]$row.currentAR -lt 1 -or
                           [int]$row.currentAR -gt [int]$type.Armor)) {
            throw "Invalid owner or armor for building $id."
        }
        $incoming[$id] = $row
        $footprints[$id] = @(Building-Footprint $row $type)
        $relocating[$id] = $isRelocating
        $edited[$id] = $isEdited
    }
    foreach ($id in $incoming.Keys) {
        if (-not $relocating[$id]) { continue }
        $type = $types[[int]$incoming[$id].buildingType]
        foreach ($key in $footprints[$id]) {
            if (-not $map.ContainsKey($key)) { throw "Building $id extends outside the map." }
            if ([int]$terrain[$map[$key].TerrainImg].type -ne [int]$type.Position -or
                [int]$terrain[$map[$key].TerrainImg].Overlapping -ne 0) {
                throw "Building $id needs compatible clear terrain at $key."
            }
            if ($blocked.ContainsKey($key)) { throw "Building $id overlaps $($blocked[$key]) at $key." }
            foreach ($other in $incoming.Keys) {
                if ($other -ne $id -and $footprints[$other] -contains $key) {
                    throw "Building $id overlaps building $other at $key."
                }
            }
        }
        if ([int]$type.UniqueOnMap -eq 1) {
            foreach ($other in $incoming.Keys) {
                if ($other -ne $id -and [int]$incoming[$other].buildingType -eq [int]$type.TypeID) {
                    throw "Building type $($type.TypeID) is unique on the map."
                }
            }
        }
    }
    $bankUnits = @{}
    if ($BridgeTerrain) {
        $unitLayers = Get-UnitLayers $metadata
        $unitTypes = Get-UnitTypes
        foreach ($layer in @("units", "reinforcements")) {
            foreach ($unit in $unitLayers.$layer) {
                $typeKey = if ($layer -eq "units") { "$($unit.unitType),$($unit.unitTL)" } else { "$($unit.type),0" }
                $definition = $unitTypes[$typeKey]
                foreach ($key in @(Building-Footprint $unit $definition)) {
                    if (-not $bankUnits.ContainsKey($key)) { $bankUnits[$key] = @() }
                    $bankUnits[$key] += $definition
                }
            }
        }
    }
    foreach ($row in $MapRows) {
        if (-not $OldMap.ContainsKey([int]$row.Id)) {
            if ([int]$terrain[$row.TerrainImg].type -notin @(3, 4, 5, 6)) {
                throw "New map tiles require ground, forest or scenery terrain."
            }
            continue
        }
        $previous = $OldMap[[int]$row.Id]
        if ($row.TerrainImg -eq $previous.TerrainImg) { continue }
        $from = $terrain[$previous.TerrainImg]
        $to = $terrain[$row.TerrainImg]
        $bankEdit = ($BridgeTerrain -eq "banks" -and [int]$from.type -in @(1, 2) -and
                    [int]$to.type -in @(1, 2) -and [int]$from.Overlapping -eq 0 -and [int]$to.Overlapping -eq 0)
        if (-not $bankEdit -and ([int]$from.type -notin @(3, 4, 5, 6) -or [int]$to.type -notin @(3, 4, 5, 6))) {
            throw "Object editor terrain edits are restricted to clear ground, forest and scenery tiles."
        }
        if ($bankEdit) {
            $key = "$($row.PosX),$($row.PosY)"
            foreach ($reservation in $metadata.blocked) {
                if ("$($reservation.x),$($reservation.y)" -ne $key -or
                    $reservation.reason -in @("Roads On Map", "Units On Map", "Reinforcement") -or
                    ($ScriptJson -and $reservation.reason -eq "Automated Units")) { continue }
                throw "Bridge bank terrain overlaps $($reservation.reason) at $key."
            }
            foreach ($road in $metadata.roads) {
                if ("$($road.PosX),$($road.PosY)" -eq $key -and [int]$road.roadtype -ne 1) {
                    throw "Bridge bank terrain requires a compatible paved road at $key."
                }
            }
            if ($bankUnits.ContainsKey($key)) {
                foreach ($definition in $bankUnits[$key]) {
                    if (-not (Unit-TerrainCompatible $definition $to)) {
                        throw "Bridge bank terrain is incompatible with a unit at $key."
                    }
                }
            }
            foreach ($id in $incoming.Keys) {
                if ($footprints[$id] -contains $key -and
                    [int]$types[[int]$incoming[$id].buildingType].Position -ne [int]$to.type) {
                    throw "Bridge bank terrain is incompatible with building $id at $key."
                }
            }
            continue
        }
        if ([int]$to.type -ne 3 -or [int]$to.Overlapping -ne 0) {
            $key = "$($row.PosX),$($row.PosY)"
            if ($blocked.ContainsKey($key)) { throw "Tree/scenery overlaps $($blocked[$key]) at $key." }
            foreach ($id in $incoming.Keys) {
                if ($footprints[$id] -contains $key) { throw "Tree/scenery overlaps building $id at $key." }
            }
        }
    }
    foreach ($id in $old.Keys) {
        if (-not $incoming.ContainsKey($id)) {
            if ($metadata.references -contains $id) {
                throw "Building $id is referenced by a mission objective or unit order; deletion is blocked."
            }
        }
    }
    $minimumNext = [Math]::Max(1000000, $metadata.nextBuildingId)
    foreach ($id in $incoming.Keys) { $minimumNext = [Math]::Max($minimumNext, $id + 1) }
    if ($NextBuildingId -lt $minimumNext) { throw "NextBuildingId must exceed all building IDs and preserve the source counter." }
    foreach ($id in $old.Keys) {
        if (-not $incoming.ContainsKey($id)) {
            $null = $Connection.Execute("DELETE FROM [Buildings On Map] WHERE [id]=$id")
        }
    }
    foreach ($id in $incoming.Keys) {
        $row = $incoming[$id]
        if (-not $edited[$id]) { continue }
        $assignments = foreach ($column in $columns | Where-Object { $_ -notin @("Counter", "id") }) {
            "[$column]=$([int]$row.$column)"
        }
        if ($old.ContainsKey($id)) {
            $null = $Connection.Execute("UPDATE [Buildings On Map] SET $($assignments -join ',') WHERE [id]=$id")
        } else {
            $names = ($columns | Where-Object { $_ -ne "Counter" } | ForEach-Object { "[$_]" }) -join ","
            $values = ($columns | Where-Object { $_ -ne "Counter" } | ForEach-Object { [int]$row.$_ }) -join ","
            $null = $Connection.Execute("INSERT INTO [Buildings On Map] ($names) VALUES ($values)")
        }
    }
    if ($NextBuildingId -ne $metadata.nextBuildingId -and
        (-not $ScriptJson -or $NextBuildingId -ne $script:originalBuildingFloor)) {
        $null = $Connection.Execute("UPDATE [Scenario Info] SET [NextID_Buildings]=$NextBuildingId")
    }
}

function Test-CroppedPoint([int]$X, [int]$Y, [int]$Size, [int]$OldSize) {
    return $X -ge 0 -and $Y -ge 0 -and $X -lt $OldSize -and $Y -lt $OldSize -and
           ($X -ge $Size -or $Y -ge $Size)
}

function Check-ResizeObjects($Connection, [int]$Size, [int]$OldSize) {
    $metadata = Get-Objects $Connection
    $buildingTypes = @{}
    foreach ($row in @(Import-Csv -LiteralPath (Join-Path $PSScriptRoot "building-types.csv"))) {
        $buildingTypes[[int]$row.TypeID] = $row
    }
    foreach ($row in $metadata.buildings) {
        foreach ($key in @(Building-Footprint $row $buildingTypes[[int]$row.buildingType])) {
            $x, $y = @($key -split ',' | ForEach-Object { [int]$_ })
            if (Test-CroppedPoint $x $y $Size $OldSize) {
                throw "Cannot shrink: building $($row.id) footprint is outside the map."
            }
        }
    }
    foreach ($key in @(Get-UnitBlocks $metadata)) {
        $x, $y = @($key -split ',' | ForEach-Object { [int]$_ })
        if (Test-CroppedPoint $x $y $Size $OldSize) {
            throw "Cannot shrink: unit/reinforcement footprint is outside the map."
        }
    }
    foreach ($row in $metadata.resizeReservations) {
        if ($row.table -eq "Taurans On Map") {
            throw "Cannot shrink: legacy Taurans On Map records require preserve-only handling."
        }
        if (Test-CroppedPoint $row.x $row.y $Size $OldSize) {
            throw "Cannot shrink: preserved $($row.table) record is outside the map."
        }
    }
    foreach ($table in @("Roads On Map", "Walls On Map", "Ruins On Map", "Mines On Map",
                          "Deployment", "Objectives Pos List", "Unit Order Pos List")) {
        foreach ($row in @(Query-Rows $Connection "SELECT [PosX],[PosY] FROM [$table]")) {
            if (Test-CroppedPoint ([int]$row.PosX) ([int]$row.PosY) $Size $OldSize) {
                throw "Cannot shrink: $table record is outside the map."
            }
        }
    }
    foreach ($row in $metadata.scripts."Automated Units") {
        if ((Test-CroppedPoint ([int]$row.StartX) ([int]$row.StartY) $Size $OldSize) -or
            (Test-CroppedPoint ([int]$row.DestX) ([int]$row.DestY) $Size $OldSize)) {
            throw "Cannot shrink: support event is outside the map."
        }
    }
}

$Source = (Resolve-Path -LiteralPath $Source).Path
$Output = [IO.Path]::GetFullPath($Output)
if (Test-Path -LiteralPath $Output) { throw "Output already exists: $Output" }
$connection = $null
$globalConnection = $null
$transaction = $false
$created = $false
$complete = $false
try {
    if ($Action -eq "Export") {
        $connection = Open-Database $Source $true
        $rows = @(Query-Rows $connection "SELECT [Id],[PosX],[PosY],[TerrainImg] FROM [Map] ORDER BY [PosY],[PosX]")
        $size = Check-Grid $rows
        $rows | Export-Csv -LiteralPath $Output -NoTypeInformation -Encoding UTF8 -NoClobber
        Write-Output "Exported $($rows.Count) cells ($size x $size) to $Output"
        return
    }
    if ($Action -eq "ExportObjects") {
        $connection = Open-Database $Source $true
        $metadata = Get-Objects $connection
        $metadata | ConvertTo-Json -Depth 8 | Out-File -LiteralPath $Output -Encoding UTF8 -NoClobber
        return
    }
    if ($Action -eq "ExportCatalog") {
        $connection = Open-Database $Source $true
        $entries = @(Query-Rows $connection "SELECT [Counter],[FileName] FROM [Scenario Info] ORDER BY [Counter]")
        ConvertTo-Json -InputObject $entries | Out-File -LiteralPath $Output -Encoding UTF8 -NoClobber
        return
    }
    if ($Action -eq "Apply") {
        if (-not $Csv -or (-not $GlobalDatabase -and -not $TerrainCsv)) {
            throw "Apply requires -Csv and either -TerrainCsv or -GlobalDatabase."
        }
        $rows = @(Import-Csv -LiteralPath $Csv)
        foreach ($column in @("Id", "PosX", "PosY", "TerrainImg")) {
            if ($rows.Count -eq 0 -or $rows[0].PSObject.Properties.Name -notcontains $column) {
                throw "CSV must contain Id, PosX, PosY, TerrainImg."
            }
        }
        $size = Check-Grid $rows
        $valid = @{}
        if ($TerrainCsv) {
            $terrains = @(Import-Csv -LiteralPath $TerrainCsv)
            if ($terrains.Count -eq 0 -or $terrains[0].PSObject.Properties.Name -notcontains "id") {
                throw "Terrain registry CSV must contain an id column."
            }
        } else {
            $globalConnection = Open-Database $GlobalDatabase $true
            $terrains = @(Query-Rows $globalConnection "SELECT [id] FROM [Terrain Types]")
        }
        foreach ($terrain in $terrains) {
            $valid[[string]$terrain.id] = $true
        }
        $ids = @{}
        foreach ($row in $rows) {
            if ([string]$row.Id -notmatch '^[0-9]+$') { throw "Id must be a nonempty nonnegative integer." }
            $id = [int]$row.Id
            if ($ids.ContainsKey($id)) { throw "Duplicate Id in CSV: $id" }
            $ids[$id] = $true
            if (-not $valid.ContainsKey([string]$row.TerrainImg) -or $row.TerrainImg -notmatch '^[A-Za-z0-9_]{1,9}$') {
                throw "Unknown/invalid terrain identifier: $($row.TerrainImg)"
            }
        }
    } else {
        if ($Member -notmatch '^[A-Za-z0-9_-]{1,9}\.[Mm][Dd][Bb]$' -or
            -not $Title -or $Title.Length -gt 50 -or $TemplateCounter -lt 1) {
            throw "Register requires a flat MDB -Member (maximum 13 chars), -Title (1..50 chars), and positive -TemplateCounter."
        }
    }
    [IO.File]::Copy($Source, $Output, $false)
    $created = $true
    $connection = Open-Database $Output $false
    $null = $connection.BeginTrans()
    $transaction = $true
    if ($Action -eq "Apply") {
        if ($BridgeTerrain -and (-not $BuildingCsv -or -not $SurfaceJson)) {
            throw "Bridge bank edits require BuildingCsv and SurfaceJson for terrain/object occupancy validation."
        }
        $existing = @(Query-Rows $connection "SELECT [Id],[PosX],[PosY],[TerrainImg] FROM [Map]")
        $oldSize = Check-Grid $existing
        $resizing = $oldSize -ne $size
        if ($resizing -and (-not $ResizeMap -or -not $BuildingCsv -or -not $SurfaceJson -or -not $UnitJson -or
                           -not $DeploymentJson -or -not $ScriptJson)) {
            throw "Resizing requires -ResizeMap square and complete object/script validation inputs."
        }
        $byId = @{}
        $byPosition = @{}
        foreach ($row in $existing) {
            $byId[[int]$row.Id] = $row
            $byPosition["$($row.PosX),$($row.PosY)"] = $row
        }
        foreach ($row in $rows) {
            $id = [int]$row.Id
            $key = "$($row.PosX),$($row.PosY)"
            if ($byPosition.ContainsKey($key)) {
                if ($id -ne [int]$byPosition[$key].Id) { throw "CSV must preserve every surviving template Id and its coordinates." }
            } elseif (-not $resizing -or $byId.ContainsKey($id)) {
                throw "New map coordinates require fresh IDs during an explicit resize."
            }
        }
        $changed = 0
        if ($ScriptJson) {
            $originalObjects = Get-Objects $connection
            $script:originalScriptReferences = $originalObjects.references
            # Computing safe fresh IDs must not normalize stock allocator values on a no-op save.
            $script:originalUnitFloor = [Math]::Max(0, $originalObjects.nextUnitId)
            $script:originalBuildingFloor = [Math]::Max(1000000, $originalObjects.nextBuildingId)
            foreach ($id in @($originalObjects.reservedIds) + @($originalObjects.references)) {
                if ($id -lt 1000000) {
                    $script:originalUnitFloor = [Math]::Max($script:originalUnitFloor, $id + 1)
                } else {
                    $script:originalBuildingFloor = [Math]::Max($script:originalBuildingFloor, $id + 1)
                }
            }
            foreach ($row in $originalObjects.buildings) {
                $script:originalBuildingFloor = [Math]::Max($script:originalBuildingFloor, [int]$row.id + 1)
            }
            Apply-Scripts $connection $size
        }
        if ($DeploymentJson) { Apply-Deployment $connection $size }
        if ($SurfaceJson) { Apply-Surfaces $connection $rows $terrains $byId }
        if ($UnitJson) { Apply-Units $connection $rows $terrains }
        if ($BuildingCsv) { Apply-Buildings $connection $rows $terrains $byId }
        if ($ScenarioJson) { Apply-Scenario $connection }
        if ($size -lt $oldSize) { Check-ResizeObjects $connection $size $oldSize }
        if ($resizing) {
            foreach ($row in $existing) {
                if ([int]$row.PosX -ge $size -or [int]$row.PosY -ge $size) {
                    $null = $connection.Execute("DELETE FROM [Map] WHERE [Id]=$([int]$row.Id)")
                }
            }
        }
        foreach ($row in $rows) {
            $id = [int]$row.Id
            if (-not $byId.ContainsKey($id)) {
                $null = $connection.Execute("INSERT INTO [Map] ([Id],[PosX],[PosY],[TerrainImg]) VALUES ($id,$([int]$row.PosX),$([int]$row.PosY),'$($row.TerrainImg)')")
                $changed++
                continue
            }
            if ([int]$row.PosX -ne [int]$byId[$id].PosX -or
                [int]$row.PosY -ne [int]$byId[$id].PosY) {
                throw "CSV must preserve every template Id and its coordinates."
            }
            if ($row.TerrainImg -ne $byId[$id].TerrainImg) {
                $sql = "UPDATE [Map] SET [TerrainImg]='$($row.TerrainImg)' WHERE [Id]=$id"
                $null = $connection.Execute($sql)
                $changed++
            }
        }
        $message = "Changed $changed terrain cells; requested object/script edits applied; unrelated records and preview blobs preserved."
    } else {
        $catalog = @(Query-Rows $connection "SELECT [Counter],[FileName] FROM [Scenario Info]")
        if (@($catalog | Where-Object { $_.FileName -ieq $Member }).Count -ne 0) {
            throw "Member is already registered: $Member"
        }
        $template = @(Query-Rows $connection "SELECT * FROM [Scenario Info] WHERE [Counter]=$TemplateCounter")
        if ($template.Count -ne 1) { throw "Expected exactly one catalog template row." }
        $fields = @("FileName", "TitleEnglish", "TitleGerman", "TitleFrench",
            "provincenameEnglish", "provincenameGerman", "provincenameFrench",
            "descriptionEnglish", "descriptionGerman", "descriptionFrench",
            "One Off Deployment", "Size")
        $columns = ($fields | ForEach-Object { "[$_]" }) -join ","
        $values = foreach ($field in $fields) {
            if ($field -eq "FileName") { "'$Member'" }
            elseif ($field -in @("TitleEnglish", "TitleGerman", "TitleFrench")) {
                "'" + $Title.Replace("'", "''") + "'"
            } else { "[$field]" }
        }
        $select = $values -join ","
        $null = $connection.Execute(
            "INSERT INTO [Scenario Info] ($columns) SELECT $select FROM [Scenario Info] WHERE [Counter]=$TemplateCounter")
        $check = @(Query-Rows $connection "SELECT [FileName] FROM [Scenario Info] WHERE [FileName]='$Member'")
        if ($check.Count -ne 1) { throw "Catalog registration did not create exactly one row." }
        if ($ScenarioDatabase) {
            $globalConnection = Open-Database $ScenarioDatabase $true
            $info = @(Query-Rows $globalConnection "SELECT * FROM [Scenario Info]")
            if ($info.Count -ne 1) { throw "Expected one scenario metadata row to register." }
            foreach ($field in $fields | Where-Object { $_ -match '^(Title|provincename|description)' }) {
                $type = $(if ($field -like "description*") { 203 } else { 202 })
                Set-ScenarioField $connection $field ([string]$info[0].$field) $type "WHERE [FileName]='$Member'"
            }
            Set-ScenarioField $connection "One Off Deployment" ([int]$info[0]."One Off Deployment") 3 "WHERE [FileName]='$Member'"
            $map = @(Query-Rows $globalConnection "SELECT [Id],[PosX],[PosY] FROM [Map]")
            Set-ScenarioField $connection "Size" (Check-Grid $map) 3 "WHERE [FileName]='$Member'"
        }
        $message = "Registered $Member as '$Title' (catalog assigns its Counter automatically)."
    }
    $connection.CommitTrans()
    $transaction = $false
    $complete = $true
    Write-Output $message
} finally {
    if ($transaction) { $connection.RollbackTrans() }
    if ($null -ne $connection -and $connection.State -ne 0) { $connection.Close() }
    if ($null -ne $globalConnection -and $globalConnection.State -ne 0) { $globalConnection.Close() }
    if ($created -and -not $complete) { [IO.File]::Delete($Output) }
}
