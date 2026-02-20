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

    /// Exclude type-only imports (show only runtime dependencies)
    #[arg(long, global = true)]
    pub runtime_only: bool,
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

    /// Compare export changes between two index snapshots
    Diff {
        /// Path to the old/baseline index database
        #[arg(long)]
        before: String,
    },

    /// List symbols in the project
    Symbols {
        /// Filter by file path
        #[arg(long)]
        file: Option<String>,
        /// Filter by symbol kind (function, class, method, etc.)
        #[arg(long)]
        kind: Option<String>,
    },

    /// Find all references to a symbol
    References {
        /// Symbol name to search for
        symbol: String,
        /// Filter by reference kind (call, type_usage, inheritance, etc.)
        #[arg(long)]
        kind: Option<String>,
        /// Filter to a specific file
        #[arg(long)]
        file: Option<String>,
    },

    /// Find all call sites of a symbol
    Callers {
        /// Symbol name to search for
        symbol: String,
        /// Filter to a specific file
        #[arg(long)]
        file: Option<String>,
    },
}

#[derive(Clone, ValueEnum)]
pub enum OutputFormat {
    Text,
    Json,
    Compact,
}
