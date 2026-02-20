use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;

use statik::cli::commands;
use statik::cli::index::run_index;
use statik::cli::output::format_index_summary;
use statik::cli::{Cli, Commands};
use statik::discovery::DiscoveryConfig;
use statik::model::Language;

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Determine project path (used by most commands)
    let project_path = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

    match cli.command {
        Commands::Index { ref path } => {
            let index_path = PathBuf::from(path)
                .canonicalize()
                .unwrap_or_else(|_| PathBuf::from(path));

            let config = build_discovery_config(&cli);
            let result = run_index(&index_path, &config)?;

            let output = format_index_summary(
                result.files_indexed + result.files_unchanged,
                result.symbols_extracted,
                result.references_found,
                result.duration_ms,
                &cli.format,
            );
            println!("{}", output);

            if !result.parse_errors.is_empty() {
                eprintln!("\nParse errors:");
                for err in &result.parse_errors {
                    eprintln!("  {}", err);
                }
            }
        }

        Commands::Deps {
            ref path,
            transitive,
            ref direction,
        } => {
            let output = commands::run_deps(
                &project_path,
                path,
                transitive,
                direction,
                cli.max_depth,
                &cli.format,
                cli.no_index,
            )?;
            println!("{}", output);
        }

        Commands::Exports { ref path } => {
            let output = commands::run_exports(&project_path, path, &cli.format, cli.no_index)?;
            println!("{}", output);
        }

        Commands::DeadCode { ref scope } => {
            let output = commands::run_dead_code(&project_path, scope, &cli.format, cli.no_index)?;
            println!("{}", output);
        }

        Commands::Cycles => {
            let output = commands::run_cycles(&project_path, &cli.format, cli.no_index)?;
            println!("{}", output);
        }

        Commands::Impact { ref path } => {
            let output = commands::run_impact(
                &project_path,
                path,
                cli.max_depth,
                &cli.format,
                cli.no_index,
            )?;
            println!("{}", output);
        }

        Commands::Summary => {
            let output = commands::run_summary(&project_path, &cli.format, cli.no_index)?;
            println!("{}", output);
        }

        Commands::Lint {
            ref config,
            ref rule,
            ref severity_threshold,
        } => {
            let (output, has_errors) = commands::run_lint(
                &project_path,
                config.as_deref(),
                rule.as_deref(),
                severity_threshold,
                &cli.format,
                cli.no_index,
            )?;
            println!("{}", output);
            if has_errors {
                std::process::exit(1);
            }
        }

        Commands::Symbols { .. } | Commands::References { .. } | Commands::Callers { .. } => {
            eprintln!("This command requires deep mode (v2). Run with --deep and ensure tsserver is installed.");
            std::process::exit(1);
        }
    }

    Ok(())
}

fn build_discovery_config(cli: &Cli) -> DiscoveryConfig {
    let languages = cli.lang.as_ref().and_then(|l| {
        Language::from_extension(match l.to_lowercase().as_str() {
            "typescript" | "ts" => "ts",
            "javascript" | "js" => "js",
            "python" | "py" => "py",
            "rust" | "rs" => "rs",
            "java" => "java",
            _ => return None,
        })
    });

    DiscoveryConfig {
        include: cli.include.clone(),
        exclude: cli.exclude.clone(),
        languages: languages.into_iter().collect(),
    }
}
