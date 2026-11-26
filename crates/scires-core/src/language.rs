//! Supported programming languages.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// A language a kernel can execute. `Julia` is the primary language.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    /// Julia — the primary language of SciRes.
    Julia,
    /// Python — machine learning use cases.
    Python,
    /// R — statistics / data analysis.
    R,
    /// C — existing algorithms.
    C,
    /// C++ — performance-critical code.
    Cpp,
    /// A language contributed by a plugin.
    #[serde(untagged)]
    Other(String),
}

impl Language {
    /// Canonical lowercase identifier.
    pub fn id(&self) -> &str {
        match self {
            Self::Julia => "julia",
            Self::Python => "python",
            Self::R => "r",
            Self::C => "c",
            Self::Cpp => "cpp",
            Self::Other(s) => s,
        }
    }

    /// Typical source file extensions.
    pub fn extensions(&self) -> &'static [&'static str] {
        match self {
            Self::Julia => &["jl"],
            Self::Python => &["py"],
            Self::R => &["r", "R"],
            Self::C => &["c", "h"],
            Self::Cpp => &["cpp", "cc", "cxx", "hpp", "hh"],
            Self::Other(_) => &[],
        }
    }

    /// Guess a built-in language from a file extension.
    pub fn from_extension(ext: &str) -> Option<Self> {
        [Self::Julia, Self::Python, Self::R, Self::C, Self::Cpp]
            .into_iter()
            .find(|l| l.extensions().contains(&ext))
    }
}

impl fmt::Display for Language {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.id())
    }
}

impl FromStr for Language {
    type Err = std::convert::Infallible;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s.to_ascii_lowercase().as_str() {
            "julia" => Self::Julia,
            "python" => Self::Python,
            "r" => Self::R,
            "c" => Self::C,
            "cpp" | "c++" => Self::Cpp,
            other => Self::Other(other.to_owned()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_displays() {
        assert_eq!("C++".parse::<Language>().unwrap(), Language::Cpp);
        assert_eq!(Language::Julia.to_string(), "julia");
        assert_eq!(
            "mojo".parse::<Language>().unwrap(),
            Language::Other("mojo".into())
        );
    }

    #[test]
    fn detects_from_extension() {
        assert_eq!(Language::from_extension("jl"), Some(Language::Julia));
        assert_eq!(Language::from_extension("hpp"), Some(Language::Cpp));
        assert_eq!(Language::from_extension("xyz"), None);
    }
}
