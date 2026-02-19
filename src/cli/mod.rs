use clap::{Parser, Subcommand, ValueEnum};

pub mod commands;
pub mod index;
pub mod output;

#[derive(Parser)]
#[command(
    name = "statik",
    version,
    about = "File-level dependency analysis for AI assistants and developers"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,

    /// Output format
    #[arg(long, global = true, default_value = "text")]
    pub format: OutputFormat,

    /// Skip auto-indexing, use existing index only
    #[arg(long, global = true)]
    pub no_index: bool,

    /// Include only files matching this glob
    #[arg(long, global = true)]
    pub include: Vec<String>,

    /// Exclude files matching this glob
    #[arg(long, global = true)]
    pub exclude: Vec<String>,

    /// Filter to specific language
    #[arg(long, global = true)]
    pub lang: Option<String>,

    /// Limit transitive depth
    #[arg(long, global = true)]
    pub max_depth: Option<usize>,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Index the project (create/update .statik/index.db)
    Index {
        /// Project path (default: current directory)
        #[arg(default_value = ".")]
        path: String,
    },

    /// File-level dependency analysis
    Deps {
        /// File path to analyze
        path: String,
        /// Follow dependencies transitively
        #[arg(long)]
        transitive: bool,
        /// Direction: in, out, or both
        #[arg(long, default_value = "both")]
        direction: String,
    },

    /// List exports from a file with used/unused status
    Exports {
        /// File or module path
        path: String,
    },

    /// Find dead code (orphaned files and unused exports)
    DeadCode {
        /// Scope: files, exports, or both
        #[arg(long, default_value = "both")]
        scope: String,
    },

    /// Detect circular dependencies
    Cycles,

    /// Blast radius / refactoring impact analysis
    Impact {
        /// File path to analyze
        path: String,
    },

    /// Project overview statistics
    Summary,

    /// Check architectural boundary rules
    Lint {
        /// Path to config file (default: .statik/rules.toml or statik.toml)
        #[arg(long)]
        config: Option<String>,
        /// Only evaluate a specific rule by ID
        #[arg(long)]
        rule: Option<String>,
        /// Minimum severity to report (error, warning, info)
        #[arg(long, default_value = "info")]
        severity_threshold: String,
    },

    // --- Deferred commands (v2+, kept hidden) ---
    /// List symbols in the project [deferred to v2]
    #[command(hide = true)]
    Symbols {
        #[arg(long)]
        file: Option<String>,
        #[arg(long)]
        kind: Option<String>,
        #[arg(long)]
        pattern: Option<String>,
    },

    /// Find all references to a symbol [deferred to v2 -- LSP handles this better]
    #[command(hide = true)]
    References {
        symbol: String,
        #[arg(long)]
        kind: Option<String>,
    },

    /// Find all callsites [deferred to v2 -- LSP handles this better]
    #[command(hide = true)]
    Callers { symbol: String },
}

#[derive(Clone, ValueEnum)]
pub enum OutputFormat {
    Text,
    Json,
    Compact,
}
