use std::path::PathBuf;

use clap::Parser;

use swe_grep::cli::{Cli, Commands};

#[test]
fn parses_context_command_defaults() {
    let cli = Cli::try_parse_from([
        "swe-grep",
        "context",
        "--symbol",
        "AuthService",
        "--path",
        ".",
    ])
    .expect("expected context command to parse");

    match cli.command {
        Commands::Context(cmd) => {
            assert_eq!(cmd.symbol, "AuthService");
            assert_eq!(cmd.depth, None);
            assert_eq!(cmd.max_lines, None);
            assert_eq!(cmd.tests, Some(false));
            assert_eq!(cmd.path, Some(PathBuf::from(".")));
        }
        other => panic!("expected context command, got {other:?}"),
    }
}

#[test]
fn parses_context_command_overrides() {
    let cli = Cli::try_parse_from([
        "swe-grep",
        "context",
        "--symbol",
        "Foo",
        "--max-lines",
        "500",
        "--include-tests",
        "--depth",
        "3",
        "--path",
        ".",
    ])
    .expect("expected context command to parse");

    match cli.command {
        Commands::Context(cmd) => {
            assert_eq!(cmd.symbol, "Foo");
            assert_eq!(cmd.depth, Some(3));
            assert_eq!(cmd.max_lines, Some(500));
            assert_eq!(cmd.tests, Some(true));
        }
        other => panic!("expected context command, got {other:?}"),
    }
}
