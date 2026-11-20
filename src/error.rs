//! zl 全体のエラー。つなぎ直せば通るもの (Transient) と、直さないと通らないもの (Permanent) を分ける。

use std::fmt;

#[derive(Debug)]
pub enum Error {
    /// サーバーに届かない、5xx、プロセスを起動できない。待ち行列に残して再試行してよい。
    Transient(String),
    /// 4xx の拒否、設定やデータの誤り。再試行しても通らない。
    Permanent(String),
    /// 利用者の入力の誤り (空の本文、登録されていない出典)。直してもらうまで通らず、待ち行列にも積まない。
    Invalid(String),
}

impl Error {
    pub fn is_transient(&self) -> bool {
        matches!(self, Error::Transient(_))
    }

    /// JSON の `kind` に出す名前。
    pub fn kind(&self) -> &'static str {
        match self {
            Error::Invalid(_) => "invalid",
            Error::Permanent(_) => "permanent",
            Error::Transient(_) => "transient",
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Transient(m) | Error::Permanent(m) | Error::Invalid(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<crate::config::ConfigError> for Error {
    fn from(e: crate::config::ConfigError) -> Self {
        Error::Permanent(e.to_string())
    }
}
