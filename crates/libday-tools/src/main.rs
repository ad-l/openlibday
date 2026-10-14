//! `libday` — inspect and export original game data.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use libday_data::{Definitions, Dir, GameData, Scenario};
use libday_formats::jet::{self, Database, Value};
use libday_formats::smacker::{self, Smacker};
use libday_formats::{Dbi, Mff, Sprite, Zip};
use libday_view::{Art, Options};
use serde_json::{Map, Value as Json};

#[derive(Parser)]
#[command(version, about)]
struct Cli {
    /// Liberation Day install directory (defaults to $LIBDAY_ROOT).
    #[arg(long, global = true)]
    root: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Show where game directories resolve.
    Info,
    /// List the sprites of a DBI file (path, or name inside IMGS).
    DbiList { file: String },
    /// Export DBI sprites as PNG files.
    DbiExport {
        file: String,
        out: PathBuf,
        /// Only export these names (default: all).
        names: Vec<String>,
    },
    /// List user tables of an Access database (`file` or `archive.zip!member`).
    MdbTables {
        file: String,
        /// Include system tables.
        #[arg(long)]
        system: bool,
    },
    /// Show the columns of a table.
    MdbSchema { file: String, table: String },
    /// Export a table as JSON lines (one object per row).
    MdbExport { file: String, table: String },
    /// List the members of a ZIP archive.
    ZipList { file: String },
    /// Dump the string table of a PE resource DLL.
    Strings { file: String },
    /// List the members of an MFF archive (`ANMSUNIT/ANMSUNIT.FF`).
    FfList {
        file: String,
        /// Only list names starting with this prefix (case-insensitive).
        #[arg(long)]
        prefix: Option<String>,
    },
    /// Show a Smacker file's header as JSON (`file.smk` or `archive.ff!member`).
    SmkInfo { file: String },
    /// Export Smacker frames as indexed PNGs (background key colour
    /// transparent) plus `meta.json`.
    SmkExport { file: String, out: PathBuf },
    /// Export every Smacker member under a prefix of an MFF archive, one
    /// sub-directory per member (path separators preserved, `.SMK` dropped).
    FfExport {
        file: String,
        out: PathBuf,
        #[arg(long, default_value = "")]
        prefix: String,
    },
    /// Re-encode a Smacker file with frames from `frame_NNN.png` files.
    ///
    /// Indexed PNGs are used as-is; other PNGs are mapped to the nearest
    /// colour of each frame's palette, with transparent pixels (alpha < 128)
    /// becoming the background key colour.
    SmkImport {
        file: String,
        frames: PathBuf,
        out: PathBuf,
    },
    /// Write a copy of an MFF archive with members replaced (`NAME=FILE`).
    FfReplace {
        file: String,
        out: PathBuf,
        #[arg(required = true)]
        replacements: Vec<String>,
    },
    /// Write a copy of a DBI with sprites replaced from PNGs (`NAME=FILE`).
    ///
    /// Transparent pixels (alpha < 128) stay transparent; colours map to the
    /// nearest DBI palette entry.
    DbiReplace {
        file: String,
        out: PathBuf,
        #[arg(required = true)]
        replacements: Vec<String>,
    },
    /// List the skirmish maps of the skirmish selector.
    Skirmishes,
    /// Print a scenario map's ordered draw list as JSON.
    MapScene(MapArgs),
    /// Render a whole scenario map to a PNG at 1:1.
    MapRender {
        #[command(flatten)]
        map: MapArgs,
        #[arg(short, long)]
        out: PathBuf,
        /// Draw tile outlines.
        #[arg(long)]
        grid: bool,
    },
}

/// Scenario selection and view options shared by the map commands.
#[derive(clap::Args)]
struct MapArgs {
    /// Scenario database name (`s1-2-1.mdb`, looked up like the game does),
    /// a path, or `archive.zip!member`.
    scenario: String,
    /// Show the human players' starting zones.
    #[arg(long)]
    deployment: bool,
    /// Bare ground only (no buildings, units, overlays, trees or scenery).
    #[arg(long)]
    terrain_only: bool,
    /// Show reinforcements arriving on this turn instead of the starting units.
    #[arg(long, default_value_t = 0)]
    arrival: i64,
    /// Hide units.
    #[arg(long)]
    no_units: bool,
}

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}

fn game(root: &Option<PathBuf>) -> Result<GameData> {
    Ok(match root {
        Some(root) => GameData::open(root)?,
        None => GameData::from_env()?,
    })
}

fn read_dbi(root: &Option<PathBuf>, file: &str) -> Result<Vec<u8>> {
    if Path::new(file).is_file() {
        return Ok(fs::read(file)?);
    }
    Ok(game(root)?.read(Dir::Imgs, file)?)
}

/// Reads `path` or `archive.zip!member`; relative paths that do not exist
/// are resolved against the game root.
fn read_spec(root: &Option<PathBuf>, spec: &str) -> Result<Vec<u8>> {
    let (path, member) = match spec.split_once('!') {
        Some((path, member)) => (path, Some(member)),
        None => (spec, None),
    };
    let mut path = PathBuf::from(path);
    if !path.exists() && path.is_relative() {
        path = game(root)?.root().join(path);
    }
    let bytes = fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(match member {
        Some(member) if is_mff(&path) => Mff::parse(&bytes)?.get(member)?.to_vec(),
        Some(member) => Zip::parse(&bytes)?.read(member)?,
        None => bytes,
    })
}

fn is_mff(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("ff"))
}

/// Resolves a plain file path against the game root when it does not exist.
fn resolve(root: &Option<PathBuf>, file: &str) -> Result<PathBuf> {
    let path = PathBuf::from(file);
    if path.exists() || !path.is_relative() {
        return Ok(path);
    }
    Ok(game(root)?.root().join(path))
}

fn split_replacement(spec: &str) -> Result<(&str, &str)> {
    spec.split_once('=')
        .ok_or_else(|| format!("expected NAME=FILE, got {spec:?}").into())
}

fn write_indexed_png(
    path: &Path,
    width: u32,
    height: u32,
    palette: &[[u8; 3]; 256],
    pixels: &[u8],
    key: Option<[u8; 3]>,
) -> Result<()> {
    let mut encoder = png::Encoder::new(fs::File::create(path)?, width, height);
    encoder.set_color(png::ColorType::Indexed);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_palette(palette.concat());
    if let Some(key) = key {
        let alpha: Vec<u8> = palette
            .iter()
            .map(|&c| if c == key { 0 } else { 255 })
            .collect();
        encoder.set_trns(alpha);
    }
    encoder.write_header()?.write_image_data(pixels)?;
    Ok(())
}

/// A decoded PNG: raw indices (8-bit indexed) or RGBA8.
enum Image {
    Indexed(Vec<u8>),
    Rgba(Vec<u8>),
}

fn read_png(path: &Path, width: usize, height: usize, keep_indices: bool) -> Result<Image> {
    let open = |t| -> Result<_> {
        let mut decoder = png::Decoder::new(fs::File::open(path)?);
        decoder.set_transformations(t);
        Ok(decoder.read_info()?)
    };
    let mut reader = open(png::Transformations::IDENTITY)?;
    let info = reader.info();
    if (info.width as usize, info.height as usize) != (width, height) {
        return Err(format!(
            "{}: expected {width}x{height}, got {}x{}",
            path.display(),
            info.width,
            info.height
        )
        .into());
    }
    if keep_indices
        && info.color_type == png::ColorType::Indexed
        && info.bit_depth == png::BitDepth::Eight
    {
        let mut buf = vec![0; reader.output_buffer_size()];
        reader.next_frame(&mut buf)?;
        buf.truncate(width * height);
        return Ok(Image::Indexed(buf));
    }
    let mut reader = open(png::Transformations::EXPAND | png::Transformations::STRIP_16)?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let frame = reader.next_frame(&mut buf)?;
    let data = &buf[..frame.buffer_size()];
    let rgba: Vec<u8> = match frame.color_type {
        png::ColorType::Rgba => data.to_vec(),
        png::ColorType::Rgb => data
            .chunks(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => data
            .chunks(2)
            .flat_map(|p| [p[0], p[0], p[0], p[1]])
            .collect(),
        png::ColorType::Grayscale => data.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        other => {
            return Err(
                format!("{}: unsupported PNG colour type {other:?}", path.display()).into(),
            );
        }
    };
    Ok(Image::Rgba(rgba))
}

/// Nearest palette entry by squared RGB distance (first exact match wins).
fn nearest(palette: &[[u8; 3]; 256], rgb: [u8; 3]) -> u8 {
    let mut best = (u32::MAX, 0u8);
    for (i, p) in palette.iter().enumerate() {
        let d: u32 = (0..3)
            .map(|c| (i32::from(p[c]) - i32::from(rgb[c])).pow(2) as u32)
            .sum();
        if d < best.0 {
            best = (d, i as u8);
            if d == 0 {
                break;
            }
        }
    }
    best.1
}

/// Header summary plus the background key colour (see `Frame::dominant_colour`).
fn smk_meta(smk: &Smacker) -> Result<Json> {
    let h = &smk.header;
    let key = smk
        .decoder()?
        .next_frame()?
        .map(|frame| frame.dominant_colour());
    Ok(serde_json::json!({
        "width": h.width,
        "height": h.height,
        "frames": h.frames,
        "frame_micros": h.frame_micros(),
        "ring_frame": h.has_ring_frame(),
        "audio": smk.has_audio(),
        "key_frames": (0..h.frames as usize).filter(|&i| smk.is_key_frame(i)).collect::<Vec<_>>(),
        "key_colour": key,
    }))
}

/// Writes indexed PNGs whose key colour is transparent (tRNS), plus `meta.json`.
fn export_smk(bytes: &[u8], out: &Path) -> Result<usize> {
    let smk = Smacker::parse(bytes)?;
    fs::create_dir_all(out)?;
    let frames = smk.decode_all()?;
    let key = frames.first().map(|f| f.dominant_colour());
    for (i, frame) in frames.iter().enumerate() {
        write_indexed_png(
            &out.join(format!("frame_{i:03}.png")),
            smk.header.width,
            smk.header.height,
            &frame.palette,
            &frame.pixels,
            key,
        )?;
    }
    fs::write(
        out.join("meta.json"),
        serde_json::to_string_pretty(&smk_meta(&smk)?)?,
    )?;
    Ok(frames.len())
}

fn json_value(value: &Value) -> Json {
    let hex = |b: &[u8]| Json::from(b.iter().map(|x| format!("{x:02X}")).collect::<String>());
    match value {
        Value::Null => Json::Null,
        Value::Bool(b) => Json::from(*b),
        Value::Byte(v) => Json::from(*v),
        Value::Int(v) => Json::from(*v),
        Value::Long(v) => Json::from(*v),
        Value::Money(v) => Json::from(format!(
            "{}{}.{:04}",
            if *v < 0 { "-" } else { "" },
            v.unsigned_abs() / 10_000,
            v.unsigned_abs() % 10_000
        )),
        Value::Float(v) => Json::from(*v as f64),
        Value::Double(v) => Json::from(*v),
        Value::DateTime(v) => {
            let (y, mo, d, h, mi, s) = jet::datetime_parts(*v);
            Json::from(format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02}"))
        }
        Value::Text(s) | Value::Memo(s) => Json::from(s.as_str()),
        Value::Binary(b) | Value::Ole(b) | Value::Raw(b) => hex(b),
        Value::Guid(g) => hex(g),
    }
}

fn write_png(path: &Path, sprite: &Sprite, rgba: &[u8]) -> Result<()> {
    let mut encoder = png::Encoder::new(
        fs::File::create(path)?,
        sprite.width as u32,
        sprite.height as u32,
    );
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(rgba)?;
    Ok(())
}

fn load_scenario(root: &Option<PathBuf>, spec: &str) -> Result<Scenario> {
    let bytes = if spec.contains('!') || Path::new(spec).is_file() {
        read_spec(root, spec)?
    } else {
        game(root)?.scenario_bytes(spec)?
    };
    Ok(Scenario::load(&Database::open(bytes)?)?)
}

fn map_scene(root: &Option<PathBuf>, map: &MapArgs) -> Result<(libday_view::Scene, Art)> {
    let game = game(root)?;
    let defs: Definitions = game.definitions()?;
    let art = Art::load(&game, &defs)?;
    let scenario = load_scenario(root, &map.scenario)?;
    let options = Options {
        arrival: (!map.no_units).then_some(map.arrival),
        deployment: map.deployment,
        terrain_only: map.terrain_only,
        ..Options::default()
    };
    let scene = libday_view::build(&scenario, &defs, &art, &options);
    for name in &scene.missing {
        eprintln!("warning: missing {name}");
    }
    Ok((scene, art))
}

fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Command::Info => {
            let game = game(&cli.root)?;
            println!("root: {}", game.root().display());
            for dir in [
                Dir::Imgs,
                Dir::Globals,
                Dir::Scenarios,
                Dir::Interf,
                Dir::AnmsUnit,
            ] {
                println!("{dir:?}: {}", game.dir(dir).display());
            }
        }
        Command::DbiList { file } => {
            let bytes = read_dbi(&cli.root, &file)?;
            let dbi = Dbi::parse(&bytes)?;
            println!(
                "{} sprites, {}x{}",
                dbi.len(),
                dbi.cell_width,
                dbi.cell_height
            );
            for name in dbi.names() {
                let status = match dbi.sprite(name) {
                    Ok(_) => "ok".to_owned(),
                    Err(err) => err.to_string(),
                };
                println!("{name}\t{status}");
            }
        }
        Command::DbiExport { file, out, names } => {
            let bytes = read_dbi(&cli.root, &file)?;
            let dbi = Dbi::parse(&bytes)?;
            fs::create_dir_all(&out)?;
            let names = if names.is_empty() {
                dbi.names().to_vec()
            } else {
                names
            };
            for name in &names {
                let sprite = dbi.sprite(name)?;
                write_png(
                    &out.join(format!("{name}.png")),
                    &sprite,
                    &sprite.to_rgba(&dbi.palette),
                )?;
            }
            println!("exported {} sprites to {}", names.len(), out.display());
        }
        Command::MdbTables { file, system } => {
            let db = Database::open(read_spec(&cli.root, &file)?)?;
            for entry in db.catalog() {
                if system || !entry.system {
                    println!("{}", entry.name);
                }
            }
        }
        Command::MdbSchema { file, table } => {
            let db = Database::open(read_spec(&cli.root, &file)?)?;
            let table = db.table(&table)?;
            println!("{} ({} rows)", table.name, table.row_count);
            for c in &table.columns {
                println!(
                    "{}\t{:?}\t{}\tflags={:#04x}",
                    c.name, c.kind, c.size, c.flags
                );
            }
        }
        Command::MdbExport { file, table } => {
            let db = Database::open(read_spec(&cli.root, &file)?)?;
            let table = db.table(&table)?;
            let mut out = std::io::BufWriter::new(std::io::stdout().lock());
            for row in db.rows(&table)? {
                let object: Map<String, Json> = table
                    .columns
                    .iter()
                    .zip(&row)
                    .map(|(c, v)| (c.name.clone(), json_value(v)))
                    .collect();
                writeln!(out, "{}", Json::Object(object))?;
            }
        }
        Command::ZipList { file } => {
            let bytes = read_spec(&cli.root, &file)?;
            for e in Zip::parse(&bytes)?.entries() {
                println!("{}\t{}\t{}\t{:08X}", e.name, e.method, e.size, e.crc32);
            }
        }
        Command::Strings { file } => {
            let bytes = read_spec(&cli.root, &file)?;
            for (id, text) in libday_formats::pe::string_table(&bytes)? {
                println!(
                    "{id}\t{}",
                    text.replace('\\', "\\\\")
                        .replace('\n', "\\n")
                        .replace('\r', "\\r")
                );
            }
        }
        Command::FfList { file, prefix } => {
            let bytes = fs::read(resolve(&cli.root, &file)?)?;
            let prefix = prefix
                .unwrap_or_default()
                .to_ascii_uppercase()
                .replace('/', "\\");
            for e in Mff::parse(&bytes)?.entries() {
                if e.name.to_ascii_uppercase().starts_with(&prefix) {
                    println!("{}\t{}", e.name, e.size);
                }
            }
        }
        Command::SmkInfo { file } => {
            let bytes = read_spec(&cli.root, &file)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&smk_meta(&Smacker::parse(&bytes)?)?)?
            );
        }
        Command::SmkExport { file, out } => {
            let n = export_smk(&read_spec(&cli.root, &file)?, &out)?;
            println!("exported {n} frames to {}", out.display());
        }
        Command::FfExport { file, out, prefix } => {
            let bytes = fs::read(resolve(&cli.root, &file)?)?;
            let mff = Mff::parse(&bytes)?;
            let prefix = prefix.to_ascii_uppercase().replace('/', "\\");
            let mut count = 0;
            for e in mff.entries() {
                let upper = e.name.to_ascii_uppercase();
                if !upper.starts_with(&prefix) || !upper.ends_with(".SMK") {
                    continue;
                }
                let stem = &e.name[..e.name.len() - 4];
                let dir = stem.split('\\').fold(out.clone(), |p, part| p.join(part));
                export_smk(mff.get(&e.name)?, &dir)?;
                count += 1;
            }
            println!("exported {count} animations to {}", out.display());
        }
        Command::SmkImport { file, frames, out } => {
            let bytes = read_spec(&cli.root, &file)?;
            let smk = Smacker::parse(&bytes)?;
            let (w, h) = (smk.header.width as usize, smk.header.height as usize);
            let decoded = smk.decode_all()?;
            let key = decoded.first().map(|f| f.dominant_colour());
            let mut pixels = Vec::with_capacity(decoded.len());
            for (i, frame) in decoded.iter().enumerate() {
                let key_index = key.map(|k| nearest(&frame.palette, k));
                pixels.push(
                    match read_png(&frames.join(format!("frame_{i:03}.png")), w, h, true)? {
                        Image::Indexed(p) => p,
                        Image::Rgba(rgba) => rgba
                            .chunks(4)
                            .map(|p| match key_index {
                                Some(k) if p[3] < 128 => k,
                                _ => nearest(&frame.palette, [p[0], p[1], p[2]]),
                            })
                            .collect(),
                    },
                );
            }
            fs::write(&out, smacker::encode(&smk, &pixels)?)?;
            println!("wrote {} ({} frames)", out.display(), pixels.len());
        }
        Command::FfReplace {
            file,
            out,
            replacements,
        } => {
            let bytes = fs::read(resolve(&cli.root, &file)?)?;
            let mff = Mff::parse(&bytes)?;
            let mut map = std::collections::HashMap::new();
            for spec in &replacements {
                let (name, path) = split_replacement(spec)?;
                let name = name.replace('/', "\\");
                let entry = mff
                    .entry(&name)
                    .ok_or_else(|| format!("no member {name}"))?;
                map.insert(entry.name.clone(), fs::read(path)?);
            }
            fs::write(&out, mff.rebuild(&map)?)?;
            println!("wrote {} ({} replaced)", out.display(), map.len());
        }
        Command::DbiReplace {
            file,
            out,
            replacements,
        } => {
            let mut bytes = read_dbi(&cli.root, &file)?;
            for spec in &replacements {
                let (name, path) = split_replacement(spec)?;
                let dbi = Dbi::parse(&bytes)?;
                let (w, h) = (dbi.cell_width as usize, dbi.cell_height as usize);
                let palette = &dbi.palette.0;
                let pixels = match read_png(Path::new(path), w, h, false)? {
                    Image::Indexed(p) => p.into_iter().map(Some).collect(),
                    Image::Rgba(rgba) => rgba
                        .chunks(4)
                        .map(|p| (p[3] >= 128).then(|| nearest(palette, [p[0], p[1], p[2]])))
                        .collect(),
                };
                let sprite = Sprite {
                    width: w,
                    height: h,
                    pixels,
                };
                bytes = dbi.replace(name, &sprite)?;
            }
            fs::write(&out, &bytes)?;
            println!("wrote {} ({} replaced)", out.display(), replacements.len());
        }
        Command::Skirmishes => {
            for entry in game(&cli.root)?.skirmishes()? {
                println!(
                    "{:>3} {:<14} {:>3}x{:<3} {:>5} pts  {}",
                    entry.counter,
                    entry.file_name,
                    entry.size,
                    entry.size,
                    entry.points,
                    entry.title
                );
            }
        }
        Command::MapScene(map) => {
            let (scene, _) = map_scene(&cli.root, &map)?;
            let items: Vec<Json> = scene
                .items
                .iter()
                .map(|item| {
                    serde_json::json!({
                        "layer": item.layer.name(),
                        "sheet": item.sheet.name(),
                        "sprite": item.sprite,
                        "x": item.x,
                        "y": item.y,
                    })
                })
                .collect();
            let json = serde_json::json!({
                "size": scene.size,
                "missing": scene.missing,
                "items": items,
            });
            println!("{}", serde_json::to_string(&json)?);
        }
        Command::MapRender { map, out, grid } => {
            let (scene, art) = map_scene(&cli.root, &map)?;
            let image = libday_view::compose(&scene, &art, grid);
            let mut encoder = png::Encoder::new(
                fs::File::create(&out)?,
                image.width as u32,
                image.height as u32,
            );
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.write_header()?.write_image_data(&image.rgba)?;
            println!("wrote {} ({}x{})", out.display(), image.width, image.height);
        }
    }
    Ok(())
}
