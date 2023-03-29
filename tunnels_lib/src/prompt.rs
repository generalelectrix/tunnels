//! Helper functions for prompting CLI input.
use io::Write;
use simple_error::SimpleError;
use std::error::Error;
use std::io;

/// Prompt the user to answer a yes or no question.
pub fn prompt_bool(msg: &str) -> Result<bool, Box<dyn Error>> {
    prompt_parse(format!("{} y/n", msg).as_str(), |input| {
        Ok(input
            .chars()
            .next()
            .and_then(|first_char| match first_char {
                'y' | 'Y' => Some(true),
                'n' | 'N' => Some(false),
                _ => None,
            })
            .ok_or_else(|| SimpleError::new("Please enter yes or no."))?)
    })
}

/// Prompt the user to enter a network port.
pub fn prompt_port() -> Result<u16, Box<dyn Error>> {
    prompt_parse("Enter a port number", |port| {
        let parsed = port.parse::<u16>()?;
        Ok(parsed)
    })
}

/// Prompt the user for input, then parse.
pub fn prompt_parse<T, F>(msg: &str, parse: F) -> Result<T, Box<dyn Error>>
where
    F: Fn(&str) -> Result<T, Box<dyn Error>>,
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
pub fn read_string() -> Result<String, Box<dyn Error>> {
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    Ok(line.trim().to_string())
}
