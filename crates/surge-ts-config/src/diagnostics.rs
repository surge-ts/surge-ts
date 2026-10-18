use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigDiagnostic {
    pub code: ConfigDiagnosticCode,
    pub message: String,
    pub file_name: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigDiagnosticCode {
    ConfigFileNotFound,
    ConfigParseError,
    ExtendsCycle,
    ExtendsFileNotFound,
    UnknownCompilerOption,
    InvalidCompilerOptionValue,
    UnsupportedLegacyCompilerOptionValue,
    UnsupportedLegacyCompilerOption,
    InvalidFilesEntry,
    InvalidIncludeEntry,
    InvalidExcludeEntry,
}

impl ConfigDiagnosticCode {
    /// A note on an option (or option value) surge does not support because
    /// TypeScript 7 removed it, which tsc reports as `TS5102`/`TS5108`.
    pub fn is_legacy_option(self) -> bool {
        matches!(self, Self::UnsupportedLegacyCompilerOption | Self::UnsupportedLegacyCompilerOptionValue)
    }
}

impl std::fmt::Display for ConfigDiagnosticCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let code = match self {
            Self::ConfigFileNotFound => "ConfigFileNotFound",
            Self::ConfigParseError => "ConfigParseError",
            Self::ExtendsCycle => "ExtendsCycle",
            Self::ExtendsFileNotFound => "ExtendsFileNotFound",
            Self::UnknownCompilerOption => "UnknownCompilerOption",
            Self::InvalidCompilerOptionValue => "InvalidCompilerOptionValue",
            Self::UnsupportedLegacyCompilerOptionValue => "UnsupportedLegacyCompilerOptionValue",
            Self::UnsupportedLegacyCompilerOption => "UnsupportedLegacyCompilerOption",
            Self::InvalidFilesEntry => "InvalidFilesEntry",
            Self::InvalidIncludeEntry => "InvalidIncludeEntry",
            Self::InvalidExcludeEntry => "InvalidExcludeEntry",
        };
        f.write_str(code)
    }
}

impl std::fmt::Display for ConfigDiagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} {}: {}",
            self.code,
            self.file_name.display(),
            self.message
        )
    }
}
