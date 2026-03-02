use clap::{Parser, Subcommand, ValueEnum};

pub mod commands;
pub mod graph_builder;
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

    /// Restrict analysis to a specific source set (defined in [scope] config)
    #[arg(long = "source-set", global = true)]
    pub source_set: Option<String>,

    /// Apply a jq filter to JSON output (implicitly sets --format json)
    #[arg(long, global = true)]
    pub jq: Option<String>,

    /// Output absolute file paths instead of project-relative paths
    #[arg(long, global = true)]
    pub absolute_paths: bool,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Index the project (create/update .statik/index.db)
    Index {
        /// Project path (default: current directory)
        #[arg(default_value = ".")]
        path: String,
        /// Force full re-index (ignore cached data)
        #[arg(long)]
        force: bool,
        /// Also index git commit history (authors, file changes)
        #[arg(long)]
        with_history: bool,
        /// Limit history to the last N commits
        #[arg(long)]
        history_depth: Option<usize>,
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

    /// Compare export changes between two snapshots (git refs or DB files)
    Diff {
        /// First git ref or DB path (baseline). When using git refs, provide two positional args.
        #[arg()]
        ref1: Option<String>,
        /// Second git ref (current). Defaults to working tree if omitted.
        #[arg()]
        ref2: Option<String>,
        /// Path to the old/baseline index database (backward-compat alternative to positional args)
        #[arg(long)]
        before: Option<String>,
        /// Compare staged changes against HEAD
        #[arg(long)]
        cached: bool,
        /// CI mode: force JSON output and use exit codes for breaking changes
        #[arg(long)]
        ci: bool,
        /// Allow breaking changes without failing (still reported in output)
        #[arg(long)]
        allow_breaking: bool,
        /// Maximum number of breaking changes before failing (0 = any breaks fail)
        #[arg(long)]
        threshold: Option<u32>,
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

    /// Analyze bus factor risk (ownership concentration + dependency fan-in)
    BusFactor {
        /// Glob pattern for files to analyze (optional)
        glob: Option<String>,
        /// Ownership threshold for counting as a contributor (0.0-1.0, default: 0.1)
        #[arg(long, default_value = "0.1")]
        threshold: f64,
        /// Recency half-life in days (default: 180)
        #[arg(long, default_value = "180")]
        half_life: f64,
        /// Half-life mode: fixed or adaptive (default: adaptive)
        #[arg(long, default_value = "adaptive")]
        half_life_mode: HalfLifeModeCli,
        /// Show per-person ownership concentration instead of per-file
        #[arg(long)]
        by_author: bool,
    },

    /// Show file ownership based on git history
    Owners {
        /// Glob pattern for files to analyze (e.g. "src/**")
        glob: String,
        /// Show only top N owners per file (default: 3)
        #[arg(long, default_value = "3")]
        top: usize,
        /// Recency half-life in days (default: 180)
        #[arg(long, default_value = "180")]
        half_life: f64,
        /// Half-life mode: fixed or adaptive (default: adaptive)
        #[arg(long, default_value = "adaptive")]
        half_life_mode: HalfLifeModeCli,
    },

    /// Analyze file change frequency and co-change patterns
    Churn {
        /// Glob pattern for files to analyze (optional)
        glob: Option<String>,
        /// Switch to co-change analysis mode (find files that change together)
        #[arg(long)]
        co_change: bool,
        /// Only show after this date (YYYY-MM-DD)
        #[arg(long)]
        since: Option<String>,
        /// Only show before this date (YYYY-MM-DD)
        #[arg(long)]
        until: Option<String>,
        /// Minimum co-change count to report (default: 3)
        #[arg(long, default_value = "3")]
        min_co_changes: usize,
    },

    /// Visualize dependency graph
    Graph {
        /// Output format: dot, svg, html (default: dot)
        #[arg(long, default_value = "dot")]
        graph_format: String,
        /// Focus on a specific file (show only its neighborhood)
        #[arg(long)]
        focus: Option<String>,
        /// Maximum depth from focus file (default: unlimited)
        #[arg(long)]
        depth: Option<usize>,
    },

    /// Impact-aware reviewer suggestion: who should review a change to this file?
    Who {
        /// File path to analyze
        path: String,
        /// Recency half-life in days (default: 180)
        #[arg(long, default_value = "180")]
        half_life: f64,
        /// Half-life mode: fixed or adaptive (default: adaptive)
        #[arg(long, default_value = "adaptive")]
        half_life_mode: HalfLifeModeCli,
        /// Show top N owners per affected file (default: 3)
        #[arg(long, default_value = "3")]
        top: usize,
    },

    /// Analyze cross-team coordination costs via ownership and dependency boundaries
    TeamCoupling {
        /// Glob pattern for files to analyze (optional)
        glob: Option<String>,
        /// Recency half-life in days (default: 180)
        #[arg(long, default_value = "180")]
        half_life: f64,
        /// Half-life mode: fixed or adaptive (default: adaptive)
        #[arg(long, default_value = "adaptive")]
        half_life_mode: HalfLifeModeCli,
        /// Ownership threshold for counting as a contributing team (0.0-1.0, default: 0.1)
        #[arg(long, default_value = "0.1")]
        threshold: f64,
    },
}

#[derive(Clone, ValueEnum)]
pub enum OutputFormat {
    Text,
    Json,
    Compact,
    Csv,
}

/// CLI enum for half-life mode selection.
#[derive(Clone, ValueEnum)]
pub enum HalfLifeModeCli {
    /// Fixed half-life (original behavior)
    Fixed,
    /// Adaptive half-life that scales with file age
    Adaptive,
}

impl HalfLifeModeCli {
    /// Convert to the analysis-layer enum.
    pub fn to_analysis_mode(&self) -> crate::analysis::ownership::HalfLifeMode {
        match self {
            HalfLifeModeCli::Fixed => crate::analysis::ownership::HalfLifeMode::Fixed,
            HalfLifeModeCli::Adaptive => crate::analysis::ownership::HalfLifeMode::Adaptive,
        }
    }
}
