//! `libday` — inspect and export original game data.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use libday_data::{Dir, GameData};
use libday_formats::{Dbi, Sprite};

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
    }
    Ok(())
}
