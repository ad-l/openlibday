//! `libday` — inspect and export original game data.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use libday_data::{Dir, GameData};
use libday_formats::jet::{self, Database, Value};
use libday_formats::{Dbi, Sprite, Zip};
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
        Some(member) => Zip::parse(&bytes)?.read(member)?,
        None => bytes,
    })
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
    }
    Ok(())
}
