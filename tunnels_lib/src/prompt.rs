//! Helper functions for prompting CLI input.
use anyhow::{anyhow, Result};
use io::Write;
use std::io;

/// Prompt the user to answer a yes or no question.
pub fn prompt_bool(msg: &str) -> Result<bool> {
    prompt_parse(format!("{} y/n", msg).as_str(), |input| {
        input
            .chars()
            .next()
            .and_then(|first_char| match first_char {
                'y' | 'Y' => Some(true),
                'n' | 'N' => Some(false),
                _ => None,
            })
            .ok_or(anyhow!("Please enter yes or no."))
    })
}

/// Prompt the user to enter a network port.
pub fn prompt_port() -> Result<u16> {
    prompt_parse("Enter a port number", |port| {
        let parsed = port.parse::<u16>()?;
        Ok(parsed)
    })
}

/// Prompt the user for input, then parse.
pub fn prompt_parse<T, F>(msg: &str, parse: F) -> Result<T>
where
    F: Fn(&str) -> Result<T>,
{
    Ok(loop {
        print!("{}: ", msg);
        io::stdout().flush()?;
        let input = read_string()?;
        match parse(&input) {
            Ok(v) => break v,
            Err(e) => {
                println!("{}", e);
            }
        }
    })
}

/// Read a line of input from stdin.
pub fn read_string() -> Result<String> {
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    Ok(line.trim().to_string())
}
