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

    /// Filter analysis to files matching this glob pattern
    #[arg(long = "path-filter", global = true)]
    pub path_filter: Option<String>,

    /// Output only the count of results (e.g. dead files, violations, cycles)
    #[arg(long, global = true)]
    pub count: bool,

    /// Limit the number of results shown
    #[arg(long, global = true)]
    pub limit: Option<usize>,

    /// Sort results by field (path, confidence, name, depth)
    #[arg(long, global = true)]
    pub sort: Option<String>,

    /// Reverse the sort order
    #[arg(long, global = true)]
    pub reverse: bool,

    /// Apply a jq filter to JSON output (implicitly sets --format json)
    #[arg(long, global = true)]
    pub jq: Option<String>,
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
        /// File path to analyze (omit when using --between)
        file: Option<String>,
        /// Follow dependencies transitively
        #[arg(long)]
        transitive: bool,
        /// Direction: in, out, or both
        #[arg(long, default_value = "both")]
        direction: String,
        /// Show edges between two glob patterns: --between <from_glob> <to_glob>
        #[arg(long, num_args = 2, value_names = ["FROM_GLOB", "TO_GLOB"])]
        between: Option<Vec<String>>,
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
    Summary {
        /// Aggregate statistics per directory
        #[arg(long)]
        by_directory: bool,
    },

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
        /// Save current violations as the baseline (suppresses them in future runs)
        #[arg(long)]
        freeze: bool,
        /// Refresh the baseline with current violations (alias for --freeze)
        #[arg(long)]
        update_baseline: bool,
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
    Csv,
}
