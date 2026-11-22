//! `tptplan`: command-line front end for tpt-planetary.
//!
//! Global flags (`--verbose`, `--format json|table`), shell completions, man
//! pages and exit codes are implemented here. `array inspect`, `array convert`,
//! `array math`, `astro convert` and `ephemeris query` are implemented in their
//! own modules.

mod array_cmd;
mod array_convert;
mod array_math;
mod astro_cmd;
mod ephemeris_cmd;
mod iso;
mod json;

use std::io::{self, Write};
use std::process::ExitCode;

use clap::{CommandFactory, Parser, Subcommand, ValueEnum};
use clap_complete::Shell;

/// Exit code: success.
pub const EXIT_OK: u8 = 0;
/// Exit code: a runtime error (I/O, invalid data, computation failure).
pub const EXIT_RUNTIME: u8 = 1;
/// Exit code: invalid usage. clap also uses 2 for argument errors.
pub const EXIT_USAGE: u8 = 2;
/// Exit code: the requested subcommand is not implemented yet.
pub const EXIT_NOT_IMPLEMENTED: u8 = 3;

/// A failed command: the exit code to use and the message for stderr.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    /// Process exit code, one of the `EXIT_*` constants.
    pub code: u8,
    /// Message printed to stderr, without the `tptplan:` prefix.
    pub message: String,
}

impl Failure {
    /// Creates a failure with the given exit code and message.
    pub fn new(code: u8, message: String) -> Self {
        Self { code, message }
    }
}

/// Output format for commands that print data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum OutputFormat {
    /// Human-readable aligned table.
    Table,
    /// Machine-readable JSON.
    Json,
}

/// tptplan: array, time and ephemeris tools for planetary-scale data.
#[derive(Debug, Parser)]
#[command(name = "tptplan", version, about, long_about = None)]
pub struct Cli {
    /// Increase diagnostic output (repeat for more: -vv).
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    pub verbose: u8,

    /// Output format for commands that print data.
    #[arg(long, global = true, value_enum, default_value_t = OutputFormat::Table)]
    pub format: OutputFormat,

    /// The subcommand to run.
    #[command(subcommand)]
    pub command: Command,
}

/// Top-level subcommands.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Work with N-dimensional arrays (Zarr, NetCDF3).
    Array {
        /// The array operation to run.
        #[command(subcommand)]
        action: ArrayCommand,
    },
    /// Time scales and coordinate frames.
    Astro {
        /// The astronomy operation to run.
        #[command(subcommand)]
        action: AstroCommand,
    },
    /// Solar-system ephemerides.
    Ephemeris {
        /// The ephemeris operation to run.
        #[command(subcommand)]
        action: EphemerisCommand,
    },
    /// Print shell completions to standard output.
    Completions {
        /// Shell to generate completions for.
        shell: Shell,
    },
    /// Write man pages into a directory.
    Man {
        /// Directory to write the generated `*.1` files into.
        #[arg(default_value = ".")]
        dir: std::path::PathBuf,
    },
}

/// `tptplan array` subcommands.
#[derive(Debug, Subcommand)]
pub enum ArrayCommand {
    /// Show shape, dtype, chunks and attributes of an array.
    Inspect {
        /// Array location: a local path, `s3://`, or `https://` URI.
        uri: String,
    },
    /// Evaluate an expression over named Zarr arrays and write a Zarr array.
    #[command(long_about = MATH_LONG_ABOUT)]
    Math {
        /// Expression to evaluate (see the grammar below).
        expression: String,
        /// Named input array, as NAME=PATH to a local Zarr array directory.
        /// Repeat for each name used in the expression.
        #[arg(long = "input", value_name = "NAME=PATH")]
        inputs: Vec<String>,
        /// Output Zarr array directory, or a NetCDF-3 file if the name ends in .nc. Must not exist yet. Written as float64.
        #[arg(short, long)]
        output: String,
        /// Zarr format of the output: 2 or 3.
        #[arg(long, value_name = "2|3", default_value_t = 2, value_parser = clap::value_parser!(u8).range(2..=3))]
        zarr_format: u8,
    },
    /// Convert between Zarr and NetCDF-3 (.nc).
    #[command(long_about = CONVERT_LONG_ABOUT)]
    Convert {
        /// Input: a .nc file, or a local Zarr array or group directory.
        input: String,
        /// Output: a Zarr directory (from a .nc input), or a .nc file (from
        /// a Zarr input). Must not exist yet.
        output: String,
        /// Zarr format written from a NetCDF-3 input: 2 or 3.
        #[arg(long, value_name = "2|3", default_value_t = 2, value_parser = clap::value_parser!(u8).range(2..=3))]
        zarr_format: u8,
    },
}

/// Long help for `array math`: the expression grammar.
const MATH_LONG_ABOUT: &str = "\
Evaluates an arithmetic expression over named Zarr arrays and writes the \
result as a float64 Zarr array. The arrays are combined lazily and written \
chunk by chunk. Operands broadcast with NumPy rules, and numeric constants \
broadcast to the shape of the other operand.

GRAMMAR
  expression := term (('+' | '-') term)*
  term       := unary (('*' | '/') unary)*
  unary      := '-' unary | primary
  primary    := NUMBER | NAME | '(' expression ')'

  NUMBER   digits with an optional fraction and exponent, such as 2, 0.5, 1e-3
  NAME     a letter or '_' then letters, digits or '_'; each name needs an
           --input NAME=PATH argument

Unary minus binds tighter than '*' and '/', so -a * b is (-a) * b. Division
follows IEEE rules. Fill-valued and NaN inputs give NaN in the output.

EXAMPLE
  tptplan array math \"(a - b) / 2 + 1\" --input a=run1/tas --input b=run2/tas \\
      --output diff.zarr

EXIT CODES
  0 success, 1 runtime error (missing or unreadable input, I/O), 2 syntax or \
usage error";

/// Long help for `array convert`.
const CONVERT_LONG_ABOUT: &str = "\
Converts between a NetCDF-3 file and a Zarr directory. The direction comes \
from the input: a .nc file is read as NetCDF-3, and a Zarr directory is read \
as Zarr (an array, or a flat group of arrays). The output format comes from \
the output path: a .nc path writes NetCDF-3, and anything else writes Zarr.

A NetCDF-3 file becomes a Zarr group, with one array per variable named after \
the variable. A Zarr array becomes a NetCDF-3 variable named after the \
directory (without a .zarr suffix). Dimension names are kept. Unnamed axes \
become dim{axis}_{length}. The _FillValue attribute and other text or numeric \
attributes are kept. Attributes are written back as double values.

SUPPORTED TYPES
  int8, int16, int32, float32, float64 (NetCDF byte, short, int, float and \
double). Unsigned and 64-bit integers, booleans and text are rejected. Variables \
must have at least one dimension. Time variables stay as numbers: CF time \
decoding is not applied. NetCDF-4 (HDF5) files are not supported.";

/// `tptplan astro` subcommands.
#[derive(Debug, Subcommand)]
pub enum AstroCommand {
    /// Convert times between time scales and coordinates between frames.
    Convert {
        /// Value to convert, for example an ISO 8601 timestamp or `ra,dec`.
        value: String,
        /// Source time scale or frame.
        #[arg(long)]
        from: String,
        /// Target time scale or frame.
        #[arg(long)]
        to: String,
        /// TT epoch (ISO 8601) for the frames that depend on time, such as
        /// `ecliptic-date` and `true-ecliptic-date`.
        #[arg(long)]
        epoch: Option<String>,
    },
}

/// `tptplan ephemeris` subcommands.
#[derive(Debug, Subcommand)]
pub enum EphemerisCommand {
    /// Query the position of a body at an epoch (requires --kernel).
    ///
    /// Bodies are NAIF IDs or one of: sun (10), earth (399), moon (301),
    /// mars (barycentre 4), jupiter (barycentre 5), saturn (barycentre 6).
    /// No other names are resolved. Output frame: icrs only.
    Query {
        /// Target body: a NAIF ID, or sun, earth, moon, mars, jupiter, saturn.
        body: String,
        /// Epoch as an ISO 8601 timestamp such as 2017-01-01T00:00:00,
        /// interpreted as TDB.
        epoch: String,
        /// Observer body: a NAIF ID or a standard name as for BODY.
        #[arg(long, default_value = "earth")]
        observer: String,
        /// Output frame (only icrs is implemented).
        #[arg(long, default_value = "icrs")]
        frame: String,
        /// Path to a DE440-style DAF/SPK file. Required.
        #[arg(long)]
        kernel: Option<std::path::PathBuf>,
    },
}

/// Prints a command's output to stdout, or its failure to stderr.
fn finish(result: Result<String, Failure>) -> ExitCode {
    match result {
        Ok(text) => {
            let mut out = io::stdout().lock();
            if writeln!(out, "{text}").and_then(|()| out.flush()).is_err() {
                return ExitCode::from(EXIT_RUNTIME);
            }
            ExitCode::from(EXIT_OK)
        }
        Err(f) => {
            eprintln!("tptplan: {}", f.message);
            ExitCode::from(f.code)
        }
    }
}

/// Reports a subcommand that is not implemented in this build.
fn write_completions(shell: Shell) -> io::Result<()> {
    let mut cmd = Cli::command();
    let name = cmd.get_name().to_string();
    clap_complete::generate(shell, &mut cmd, name, &mut io::stdout());
    io::stdout().flush()
}

fn write_man_pages(dir: &std::path::Path) -> io::Result<usize> {
    std::fs::create_dir_all(dir)?;
    let root = Cli::command();
    let mut written = 0usize;
    let mut stack = vec![root];
    while let Some(cmd) = stack.pop() {
        let name = cmd.get_name().to_string();
        let mut buf: Vec<u8> = Vec::new();
        clap_mangen::Man::new(cmd.clone()).render(&mut buf)?;
        let file = dir.join(format!("{}.1", name.replace(' ', "-")));
        std::fs::write(&file, &buf)?;
        written += 1;
        for sub in cmd.get_subcommands() {
            // Each subcommand gets its own page, named after its full path.
            let full = format!("{}-{}", name, sub.get_name());
            let renamed = sub.clone().name(full);
            stack.push(renamed);
        }
    }
    Ok(written)
}

fn run(cli: Cli) -> ExitCode {
    let verbose = cli.verbose;
    let format = cli.format;
    match cli.command {
        Command::Completions { shell } => match write_completions(shell) {
            Ok(()) => ExitCode::from(EXIT_OK),
            Err(e) => {
                eprintln!("tptplan: cannot write completions: {e}");
                ExitCode::from(EXIT_RUNTIME)
            }
        },
        Command::Man { dir } => match write_man_pages(&dir) {
            Ok(n) => {
                if verbose > 0 {
                    eprintln!("tptplan: wrote {n} man pages to {}", dir.display());
                }
                ExitCode::from(EXIT_OK)
            }
            Err(e) => {
                eprintln!("tptplan: cannot write man pages: {e}");
                ExitCode::from(EXIT_RUNTIME)
            }
        },
        Command::Array { action } => match action {
            ArrayCommand::Inspect { uri } => finish(array_cmd::inspect(&uri, format, verbose)),
            ArrayCommand::Math {
                expression,
                inputs,
                output,
                zarr_format,
            } => finish(array_math::math(
                &expression,
                &inputs,
                &output,
                zarr_format,
                verbose,
            )),
            ArrayCommand::Convert {
                input,
                output,
                zarr_format,
            } => finish(array_convert::convert(
                &input,
                &output,
                zarr_format,
                verbose,
            )),
        },
        Command::Astro { action } => match action {
            AstroCommand::Convert {
                value,
                from,
                to,
                epoch,
            } => finish(astro_cmd::convert_command_at(
                &value,
                &from,
                &to,
                epoch.as_deref(),
                format,
            )),
        },
        Command::Ephemeris { action } => match action {
            EphemerisCommand::Query {
                body,
                epoch,
                observer,
                frame,
                kernel,
            } => finish(ephemeris_cmd::query(
                &ephemeris_cmd::QueryArgs {
                    body: &body,
                    epoch: &epoch,
                    observer: &observer,
                    frame: &frame,
                    kernel: kernel.as_deref(),
                },
                format,
            )),
        },
    }
}

fn main() -> ExitCode {
    run(Cli::parse())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn global_flags_parse_after_subcommand() {
        let cli = Cli::try_parse_from([
            "tptplan", "array", "inspect", "x.zarr", "-vv", "--format", "json",
        ])
        .expect("valid arguments");
        assert_eq!(cli.verbose, 2);
        assert_eq!(cli.format, OutputFormat::Json);
    }

    #[test]
    fn man_pages_are_written() {
        let dir = std::env::temp_dir().join(format!("tptplan-man-test-{}", std::process::id()));
        let n = write_man_pages(&dir).expect("man pages");
        assert!(n > 5);
        assert!(dir.join("tptplan.1").exists());
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }
}
