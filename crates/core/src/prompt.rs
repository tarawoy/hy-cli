use anyhow::Result;
use std::io::{self, Write};

pub fn prompt(msg: &str) -> Result<String> {
    let mut stdout = io::stdout();
    write!(stdout, "{msg}")?;
    stdout.flush()?;
    let mut s = String::new();
    io::stdin().read_line(&mut s)?;
    Ok(s.trim().to_string())
}

pub fn prompt_optional(msg: &str) -> Result<Option<String>> {
    let s = prompt(msg)?;
    if s.is_empty() { Ok(None) } else { Ok(Some(s)) }
}
