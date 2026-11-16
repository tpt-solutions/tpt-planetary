//! Target triple parsing and representation.
//!
//! This crate is an MIT-licensed, API-compatible subset of the `target-lexicon`
//! crate. It parses and formats target triples such as `x86_64-unknown-linux-gnu`
//! and `x86_64-pc-windows-msvc`, and reports the host triple.
//!
//! Components that are not recognised are kept as `Other(String)` rather than
//! rejected, so unusual or newer triples still round-trip through `Display`.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::fmt;
use std::str::FromStr;

/// Error returned when a string is not a valid target triple.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    message: String,
}

impl ParseError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ParseError {}

/// The 32-bit x86 family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum X86_32Architecture {
    /// `i386`
    I386,
    /// `i586`
    I586,
    /// `i686`
    I686,
}

/// CPU architecture.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[allow(non_camel_case_types)]
pub enum Architecture {
    /// `arm`
    Arm,
    /// `armv7`
    Armv7,
    /// `aarch64`
    Aarch64,
    /// `loongarch64`
    Loongarch64,
    /// `mips`
    Mips,
    /// `mips64`
    Mips64,
    /// `powerpc`
    Powerpc,
    /// `powerpc64`
    Powerpc64,
    /// `powerpc64le`
    Powerpc64le,
    /// `riscv64`
    Riscv64,
    /// `s390x`
    S390x,
    /// `sparc64`
    Sparc64,
    /// `wasm32`
    Wasm32,
    /// `wasm64`
    Wasm64,
    /// `i386`, `i586` or `i686`
    X86_32(X86_32Architecture),
    /// `x86_64`
    X86_64,
    /// Any architecture name not listed above.
    Other(String),
}

/// Vendor component of a triple.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Vendor {
    /// `unknown`
    Unknown,
    /// `apple`
    Apple,
    /// `pc`
    Pc,
    /// `nvidia`
    Nvidia,
    /// Any other vendor string.
    Custom(String),
}

impl Vendor {
    /// Returns the vendor as it appears in a triple.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Unknown => "unknown",
            Self::Apple => "apple",
            Self::Pc => "pc",
            Self::Nvidia => "nvidia",
            Self::Custom(s) => s,
        }
    }
}

/// Operating system component of a triple.
///
/// Variants that carry a version (`Darwin`, `IOS`) hold it as the text that
/// follows the OS name, e.g. `darwin21.0` is `Darwin(Some("21.0"))`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[allow(clippy::upper_case_acronyms)]
pub enum OperatingSystem {
    /// `unknown`
    Unknown,
    /// `aix`
    Aix,
    /// `cygwin`
    Cygwin,
    /// `darwin`, optionally with a version suffix.
    Darwin(Option<String>),
    /// `dragonfly`
    Dragonfly,
    /// `emscripten`
    Emscripten,
    /// `freebsd`
    Freebsd,
    /// `haiku`
    Haiku,
    /// `hurd`
    Hurd,
    /// `ios`, optionally with a version suffix.
    IOS(Option<String>),
    /// `linux`
    Linux,
    /// `netbsd`
    Netbsd,
    /// `openbsd`
    Openbsd,
    /// `solaris`
    Solaris,
    /// `wasi`
    Wasi,
    /// `windows`
    Windows,
    /// Any other OS name.
    Other(String),
}

/// Environment (ABI) component of a triple.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Environment {
    /// `unknown`
    Unknown,
    /// `android`
    Android,
    /// `androideabi`
    Androideabi,
    /// `eabi`
    Eabi,
    /// `eabihf`
    Eabihf,
    /// `gnu`
    Gnu,
    /// `gnueabi`
    Gnueabi,
    /// `gnueabihf`
    Gnueabihf,
    /// `gnux32`
    Gnux32,
    /// `musl`
    Musl,
    /// `musleabi`
    Musleabi,
    /// `musleabihf`
    Musleabihf,
    /// `msvc`
    Msvc,
    /// Any other environment name.
    Other(String),
}

/// Object file format implied by a triple.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BinaryFormat {
    /// Format not known.
    Unknown,
    /// COFF (Windows).
    Coff,
    /// ELF (most Unix-like systems).
    Elf,
    /// Mach-O (Apple platforms).
    Macho,
    /// WebAssembly.
    Wasm,
    /// XCOFF (AIX).
    Xcoff,
}

/// A parsed target triple.
///
/// Two triples are equal when all components are equal. `binary_format` is
/// derived from the operating system and architecture when parsing.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Triple {
    /// CPU architecture.
    pub architecture: Architecture,
    /// Vendor.
    pub vendor: Vendor,
    /// Operating system.
    pub operating_system: OperatingSystem,
    /// Environment (ABI).
    pub environment: Environment,
    /// Object file format.
    pub binary_format: BinaryFormat,
}

/// Builds a [`Triple`] from a string literal, panicking if it is invalid.
///
/// Intended for constants and tests.
///
/// ```
/// use target_lexicon::triple;
/// let t = triple!("x86_64-unknown-linux-gnu");
/// assert_eq!(t.to_string(), "x86_64-unknown-linux-gnu");
/// ```
#[macro_export]
macro_rules! triple {
    ($s:expr) => {
        <$crate::Triple as ::core::str::FromStr>::from_str($s).expect("invalid target triple")
    };
}

impl Triple {
    /// Returns the triple for the platform this code was compiled for.
    ///
    /// Build scripts run on the host, so this is the host triple.
    #[must_use]
    pub fn host() -> Self {
        let os_name = match std::env::consts::OS {
            "macos" => "darwin",
            "android" => "linux",
            other => other,
        };
        let arch_name = match std::env::consts::ARCH {
            "x86" => "i686",
            other => other,
        };
        let vendor = match std::env::consts::OS {
            "macos" | "ios" => Vendor::Apple,
            "windows" => Vendor::Pc,
            _ => Vendor::Unknown,
        };
        let environment = if std::env::consts::OS == "android" {
            if cfg!(target_arch = "arm") {
                Environment::Androideabi
            } else {
                Environment::Android
            }
        } else if cfg!(target_env = "msvc") {
            Environment::Msvc
        } else if cfg!(target_env = "musl") {
            Environment::Musl
        } else if cfg!(target_env = "gnu") {
            Environment::Gnu
        } else {
            Environment::Unknown
        };
        let mut triple = Self {
            architecture: parse_architecture(arch_name),
            vendor,
            operating_system: parse_operating_system(os_name),
            environment,
            binary_format: BinaryFormat::Unknown,
        };
        triple.binary_format =
            default_binary_format(&triple.operating_system, &triple.architecture);
        triple
    }
}

impl FromStr for Triple {
    type Err = ParseError;

    /// Parses a triple of the form `arch[-vendor]-os[-env]`.
    ///
    /// Three components are read as `arch-vendor-os`, unless the second and
    /// third are a known OS and a known environment, in which case they are
    /// read as `arch-os-env`.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = s.split('-').collect();
        if parts.iter().any(|p| p.is_empty()) {
            return Err(ParseError::new(format!(
                "invalid target triple {s:?}: empty component"
            )));
        }
        let (arch, vendor, os, env) = match parts.as_slice() {
            [arch] => (*arch, "unknown", "unknown", None),
            [arch, os] => (*arch, "unknown", *os, None),
            [arch, vendor, os] if is_known_os(vendor) && is_known_env(os) => {
                (*arch, "unknown", *vendor, Some(*os))
            }
            [arch, vendor, os] => (*arch, *vendor, *os, None),
            [arch, vendor, os, env] => (*arch, *vendor, *os, Some(*env)),
            _ => {
                return Err(ParseError::new(format!(
                    "invalid target triple {s:?}: expected at most 4 components, found {}",
                    parts.len()
                )));
            }
        };

        let operating_system = parse_operating_system(os);
        let mut triple = Self {
            architecture: parse_architecture(arch),
            vendor: parse_vendor(vendor),
            environment: env.map_or(Environment::Unknown, parse_environment),
            binary_format: BinaryFormat::Unknown,
            operating_system,
        };
        triple.binary_format =
            default_binary_format(&triple.operating_system, &triple.architecture);
        Ok(triple)
    }
}

impl fmt::Display for Triple {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}-{}-{}",
            self.architecture, self.vendor, self.operating_system
        )?;
        if self.environment != Environment::Unknown {
            write!(f, "-{}", self.environment)?;
        }
        Ok(())
    }
}

impl fmt::Display for Architecture {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Arm => "arm",
            Self::Armv7 => "armv7",
            Self::Aarch64 => "aarch64",
            Self::Loongarch64 => "loongarch64",
            Self::Mips => "mips",
            Self::Mips64 => "mips64",
            Self::Powerpc => "powerpc",
            Self::Powerpc64 => "powerpc64",
            Self::Powerpc64le => "powerpc64le",
            Self::Riscv64 => "riscv64",
            Self::S390x => "s390x",
            Self::Sparc64 => "sparc64",
            Self::Wasm32 => "wasm32",
            Self::Wasm64 => "wasm64",
            Self::X86_32(X86_32Architecture::I386) => "i386",
            Self::X86_32(X86_32Architecture::I586) => "i586",
            Self::X86_32(X86_32Architecture::I686) => "i686",
            Self::X86_64 => "x86_64",
            Self::Other(s) => s,
        };
        f.write_str(name)
    }
}

impl fmt::Display for Vendor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl fmt::Display for OperatingSystem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Unknown => "unknown",
            Self::Aix => "aix",
            Self::Cygwin => "cygwin",
            Self::Darwin(version) => {
                return write!(f, "darwin{}", version.as_deref().unwrap_or(""));
            }
            Self::Dragonfly => "dragonfly",
            Self::Emscripten => "emscripten",
            Self::Freebsd => "freebsd",
            Self::Haiku => "haiku",
            Self::Hurd => "hurd",
            Self::IOS(version) => {
                return write!(f, "ios{}", version.as_deref().unwrap_or(""));
            }
            Self::Linux => "linux",
            Self::Netbsd => "netbsd",
            Self::Openbsd => "openbsd",
            Self::Solaris => "solaris",
            Self::Wasi => "wasi",
            Self::Windows => "windows",
            Self::Other(s) => s,
        };
        f.write_str(name)
    }
}

impl fmt::Display for Environment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Unknown => "unknown",
            Self::Android => "android",
            Self::Androideabi => "androideabi",
            Self::Eabi => "eabi",
            Self::Eabihf => "eabihf",
            Self::Gnu => "gnu",
            Self::Gnueabi => "gnueabi",
            Self::Gnueabihf => "gnueabihf",
            Self::Gnux32 => "gnux32",
            Self::Musl => "musl",
            Self::Musleabi => "musleabi",
            Self::Musleabihf => "musleabihf",
            Self::Msvc => "msvc",
            Self::Other(s) => s,
        };
        f.write_str(name)
    }
}

fn parse_architecture(s: &str) -> Architecture {
    match s {
        "arm" => Architecture::Arm,
        "armv7" => Architecture::Armv7,
        "aarch64" => Architecture::Aarch64,
        "loongarch64" => Architecture::Loongarch64,
        "mips" => Architecture::Mips,
        "mips64" => Architecture::Mips64,
        "powerpc" => Architecture::Powerpc,
        "powerpc64" => Architecture::Powerpc64,
        "powerpc64le" => Architecture::Powerpc64le,
        "riscv64" | "riscv64gc" => Architecture::Riscv64,
        "s390x" => Architecture::S390x,
        "sparc64" => Architecture::Sparc64,
        "wasm32" => Architecture::Wasm32,
        "wasm64" => Architecture::Wasm64,
        "i386" => Architecture::X86_32(X86_32Architecture::I386),
        "i586" => Architecture::X86_32(X86_32Architecture::I586),
        "i686" => Architecture::X86_32(X86_32Architecture::I686),
        "x86_64" => Architecture::X86_64,
        other => Architecture::Other(other.to_string()),
    }
}

fn parse_vendor(s: &str) -> Vendor {
    match s {
        "unknown" => Vendor::Unknown,
        "apple" => Vendor::Apple,
        "pc" => Vendor::Pc,
        "nvidia" => Vendor::Nvidia,
        other => Vendor::Custom(other.to_string()),
    }
}

/// Parses `darwin`/`ios` with an optional version suffix such as `darwin21.0`.
fn strip_version(s: &str, prefix: &str) -> Option<Option<String>> {
    let rest = s.strip_prefix(prefix)?;
    if rest.is_empty() {
        Some(None)
    } else if rest.starts_with(|c: char| c.is_ascii_digit()) {
        Some(Some(rest.to_string()))
    } else {
        None
    }
}

fn parse_operating_system(s: &str) -> OperatingSystem {
    if let Some(version) = strip_version(s, "darwin") {
        return OperatingSystem::Darwin(version);
    }
    if let Some(version) = strip_version(s, "ios") {
        return OperatingSystem::IOS(version);
    }
    match s {
        "unknown" => OperatingSystem::Unknown,
        "aix" => OperatingSystem::Aix,
        "cygwin" => OperatingSystem::Cygwin,
        "dragonfly" => OperatingSystem::Dragonfly,
        "emscripten" => OperatingSystem::Emscripten,
        "freebsd" => OperatingSystem::Freebsd,
        "haiku" => OperatingSystem::Haiku,
        "hurd" => OperatingSystem::Hurd,
        "linux" => OperatingSystem::Linux,
        "netbsd" => OperatingSystem::Netbsd,
        "openbsd" => OperatingSystem::Openbsd,
        "solaris" => OperatingSystem::Solaris,
        "wasi" => OperatingSystem::Wasi,
        "windows" => OperatingSystem::Windows,
        other => OperatingSystem::Other(other.to_string()),
    }
}

fn parse_environment(s: &str) -> Environment {
    match s {
        "unknown" => Environment::Unknown,
        "android" => Environment::Android,
        "androideabi" => Environment::Androideabi,
        "eabi" => Environment::Eabi,
        "eabihf" => Environment::Eabihf,
        "gnu" => Environment::Gnu,
        "gnueabi" => Environment::Gnueabi,
        "gnueabihf" => Environment::Gnueabihf,
        "gnux32" => Environment::Gnux32,
        "musl" => Environment::Musl,
        "musleabi" => Environment::Musleabi,
        "musleabihf" => Environment::Musleabihf,
        "msvc" => Environment::Msvc,
        other => Environment::Other(other.to_string()),
    }
}

fn is_known_os(s: &str) -> bool {
    !matches!(
        parse_operating_system(s),
        OperatingSystem::Other(_) | OperatingSystem::Unknown
    )
}

fn is_known_env(s: &str) -> bool {
    !matches!(
        parse_environment(s),
        Environment::Other(_) | Environment::Unknown
    )
}

fn default_binary_format(os: &OperatingSystem, arch: &Architecture) -> BinaryFormat {
    match os {
        OperatingSystem::Windows | OperatingSystem::Cygwin => BinaryFormat::Coff,
        OperatingSystem::Darwin(_) | OperatingSystem::IOS(_) => BinaryFormat::Macho,
        OperatingSystem::Aix => BinaryFormat::Xcoff,
        OperatingSystem::Emscripten | OperatingSystem::Wasi => BinaryFormat::Wasm,
        OperatingSystem::Unknown | OperatingSystem::Other(_) => {
            if matches!(arch, Architecture::Wasm32 | Architecture::Wasm64) {
                BinaryFormat::Wasm
            } else {
                BinaryFormat::Unknown
            }
        }
        _ => BinaryFormat::Elf,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROUND_TRIPS: &[&str] = &[
        "x86_64-unknown-linux-gnu",
        "x86_64-unknown-linux-musl",
        "x86_64-pc-windows-msvc",
        "i686-pc-windows-msvc",
        "i686-pc-windows-gnu",
        "x86_64-apple-darwin",
        "aarch64-apple-ios",
        "aarch64-apple-darwin",
        "aarch64-unknown-linux-gnu",
        "arm-unknown-linux-gnueabihf",
        "armv7-unknown-linux-gnueabihf",
        "aarch64-unknown-linux-android",
        "armv7-unknown-linux-androideabi",
        "wasm32-unknown-emscripten",
        "wasm32-unknown-unknown",
        "wasm32-unknown-wasi",
        "riscv64-unknown-linux-gnu",
        "x86_64-pc-cygwin",
        "x86_64-unknown-freebsd",
        "powerpc64le-unknown-linux-gnu",
        "x86_64-custom-otheros-somenv",
    ];

    #[test]
    fn round_trips_through_display() {
        for s in ROUND_TRIPS {
            let t: Triple = s.parse().unwrap_or_else(|e| panic!("{s}: {e}"));
            assert_eq!(t.to_string(), *s, "round trip of {s}");
        }
    }

    #[test]
    fn components_are_parsed() {
        let t = triple!("i686-pc-windows-msvc");
        assert_eq!(
            t.architecture,
            Architecture::X86_32(X86_32Architecture::I686)
        );
        assert_eq!(t.vendor, Vendor::Pc);
        assert_eq!(t.operating_system, OperatingSystem::Windows);
        assert_eq!(t.environment, Environment::Msvc);
        assert_eq!(t.binary_format, BinaryFormat::Coff);
    }

    #[test]
    fn three_components_read_as_arch_vendor_os_unless_os_env_form() {
        let vendor_form = triple!("x86_64-pc-windows");
        assert_eq!(vendor_form.vendor, Vendor::Pc);
        assert_eq!(vendor_form.operating_system, OperatingSystem::Windows);
        assert_eq!(vendor_form.environment, Environment::Unknown);

        let env_form = triple!("arm-linux-gnueabihf");
        assert_eq!(env_form.vendor, Vendor::Unknown);
        assert_eq!(env_form.operating_system, OperatingSystem::Linux);
        assert_eq!(env_form.environment, Environment::Gnueabihf);
    }

    #[test]
    fn darwin_and_ios_keep_version_suffix() {
        let t = triple!("aarch64-apple-darwin21.0");
        assert_eq!(
            t.operating_system,
            OperatingSystem::Darwin(Some("21.0".to_string()))
        );
        assert_eq!(t.binary_format, BinaryFormat::Macho);
        assert_eq!(t.to_string(), "aarch64-apple-darwin21.0");

        assert_eq!(
            triple!("aarch64-apple-ios").operating_system,
            OperatingSystem::IOS(None)
        );
    }

    #[test]
    fn vendor_as_str_matches_triple_text() {
        assert_eq!(triple!("i686-win7-windows-msvc").vendor.as_str(), "win7");
    }

    #[test]
    fn equal_strings_give_equal_triples() {
        assert_eq!(
            triple!("x86_64-unknown-linux-gnu"),
            triple!("x86_64-unknown-linux-gnu")
        );
        assert_ne!(
            triple!("x86_64-unknown-linux-gnu"),
            triple!("x86_64-unknown-linux-musl")
        );
    }

    #[test]
    fn rejects_empty_components_and_too_many_parts() {
        assert!("".parse::<Triple>().is_err());
        assert!("x86_64--linux".parse::<Triple>().is_err());
        assert!("a-b-c-d-e".parse::<Triple>().is_err());
    }

    #[test]
    fn host_is_stable_and_parses_back() {
        let host = Triple::host();
        assert_eq!(host, Triple::host());
        let reparsed: Triple = host.to_string().parse().unwrap();
        assert_eq!(reparsed.architecture, host.architecture);
        assert_eq!(reparsed.operating_system, host.operating_system);
    }
}
