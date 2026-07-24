use anyhow::Result;
use std::time::SystemTime;

const FACTS: &[&str] = &[
    "honey never spoils. archaeologists have found edible honey in 3,000-year-old egyptian pots.",
    "octopuses have three hearts, nine brains, and blue blood.",
    "a group of flamingos is called a 'flamboyance'.",
    "bananas are berries, but strawberries are not.",
    "the shortest commercial flight in the world lasts about 90 seconds.",
    "wombat poop is cube-shaped, which helps it stay in place on rocks.",
    "there are more trees on earth than stars in the milky way galaxy.",
    "the first computer bug was an actual moth found in a harvard mark ii relay in 1947.",
    "the first message sent over ARPANET was meant to be 'LOGIN', but only 'LO' made it through before a crash.",
    "ipv6 addresses are 128 bits, providing about 340 undecillion unique addresses.",
    "the 'sudo' command name stands for 'superuser do'.",
];

/// Pick a fun fact using low-quality time-based randomness.
///
/// Good enough for an easter egg; pull in `rand` if this ever needs to feel
/// truly random.
pub(crate) fn pick_fact() -> &'static str {
    let elapsed = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("system time should be after the unix epoch");
    let index = (elapsed.as_nanos() % FACTS.len() as u128) as usize;
    FACTS[index]
}

pub(crate) fn funfact_cmd() -> Result<()> {
    println!("{}", pick_fact());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{Cli, Commands};
    use clap::{CommandFactory, Parser};

    #[test]
    fn pick_fact_returns_a_known_fact() {
        let fact = pick_fact();
        assert!(FACTS.contains(&fact), "picked fact should be in the list");
    }

    #[test]
    fn funfact_cli_parses() {
        let cli = Cli::try_parse_from(["mew", "funfact"]).unwrap();
        assert!(matches!(cli.command, Some(Commands::Funfact)));
    }

    #[test]
    fn funfact_is_hidden_from_help() {
        let mut cmd = Cli::command();
        let help = cmd.render_help().to_string();
        assert!(
            !help.contains("funfact"),
            "hidden subcommand should not appear in --help"
        );
    }
}
