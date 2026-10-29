//! Server settings, project overrides and latexmk configuration.
pub(crate) mod project_config;

use serde::{Deserialize, Serialize};
use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
};

mod latexmk;
mod latexmk_native;

const DEFAULT_OUTPUT_DIRECTORY: &str = "build/texquill";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Engine {
    PdfLatex,
    XeLatex,
    LuaLatex,
}

impl Engine {
    pub fn from_program(program: &str) -> Option<Self> {
        Some(match program.to_ascii_lowercase().as_str() {
            "pdflatex" => Self::PdfLatex,
            "xelatex" => Self::XeLatex,
            "lualatex" => Self::LuaLatex,
            _ => return None,
        })
    }

    fn latexmk_argument(self) -> &'static str {
        match self {
            Self::PdfLatex => "-pdf",
            Self::XeLatex => "-xelatex",
            Self::LuaLatex => "-lualatex",
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct Config {
    pub build: BuildConfig,
    pub formatting: FormattingConfig,
    pub distribution: DistributionConfig,
    pub diagnostics: DiagnosticsConfig,
    pub indexing: IndexingConfig,
    pub inlay_hints: InlayHintsConfig,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct InlayHintsConfig {
    pub enabled: bool,
    pub numbers: bool,
    pub pages: bool,
    pub citations: bool,
}
impl Default for InlayHintsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            numbers: true,
            pages: false,
            citations: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct IndexingConfig {
    pub max_files: usize,
    pub max_file_size: u64,
    pub max_total_bytes: u64,
    pub max_depth: usize,
    pub completion_limit: usize,
    pub exclude: Vec<String>,
}
impl Default for IndexingConfig {
    fn default() -> Self {
        Self {
            max_files: 1024,
            max_file_size: 2 * 1024 * 1024,
            max_total_bytes: 64 * 1024 * 1024,
            max_depth: 16,
            completion_limit: 256,
            exclude: ["target", "build", "node_modules", "vendor"]
                .map(String::from)
                .to_vec(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct DistributionConfig {
    pub enabled: bool,
    pub executable: String,
    pub roots: Vec<PathBuf>,
}

impl Default for DistributionConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            executable: "kpsewhich".into(),
            roots: vec![],
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct DiagnosticsConfig {
    pub math_delimiters: bool,
    pub duplicate_bibliography_fields: bool,
    pub undefined_references: bool,
    pub undefined_citations: bool,
    pub undefined_glossary: bool,
    pub undefined_indexes: bool,
    pub chktex: ChktexConfig,
}
impl Default for DiagnosticsConfig {
    fn default() -> Self {
        Self {
            math_delimiters: false,
            duplicate_bibliography_fields: true,
            undefined_references: true,
            undefined_citations: true,
            undefined_glossary: true,
            undefined_indexes: true,
            chktex: ChktexConfig::default(),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct ChktexConfig {
    pub enabled: bool,
    pub executable: String,
    pub arguments: Vec<String>,
    pub timeout_seconds: u64,
}
impl Default for ChktexConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            executable: "chktex".into(),
            arguments: vec![],
            timeout_seconds: 10,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
pub enum Formatter {
    #[serde(rename = "latexindent")]
    Latexindent,
    #[default]
    #[serde(rename = "tex-fmt")]
    TexFmt,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct FormattingConfig {
    pub on_type: bool,
    pub formatter: Formatter,
    pub executable: Option<String>,
    pub arguments: Option<Vec<String>>,
    pub timeout_seconds: u64,
}

impl Default for FormattingConfig {
    fn default() -> Self {
        Self {
            on_type: true,
            formatter: Formatter::TexFmt,
            executable: None,
            arguments: None,
            timeout_seconds: 10,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildResolution {
    pub sources: Vec<PathBuf>,
    pub issues: Vec<String>,
    pub native: bool,
    #[serde(skip)]
    pub inferred: Vec<String>,
    #[serde(skip)]
    pub dependencies: Vec<PathBuf>,
    #[serde(skip)]
    pub rebuild: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct BuildConfig {
    #[serde(skip)]
    pub resolution: BuildResolution,
    pub bibliography_tool: Option<BibliographyToolConfig>,
    pub index_tools: Vec<IndexToolConfig>,
    pub executable: String,
    pub arguments: Option<Vec<String>>,
    pub rebuild_arguments: Option<Vec<String>>,
    pub engine: Option<Engine>,
    pub output_directory: Option<String>,
    pub auxiliary_directory: Option<String>,
    pub final_output_directory: Option<String>,
    pub root_file: Option<String>,
    pub job_name: Option<String>,
    pub on_save: bool,
    pub timeout_seconds: u64,
}

impl Default for BuildConfig {
    fn default() -> Self {
        Self {
            resolution: BuildResolution::default(),
            bibliography_tool: None,
            index_tools: vec![],
            executable: "latexmk".into(),
            arguments: None,
            rebuild_arguments: None,
            engine: None,
            output_directory: None,
            auxiliary_directory: None,
            final_output_directory: None,
            root_file: None,
            job_name: None,
            on_save: false,
            timeout_seconds: 120,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum BibliographyBackend {
    #[default]
    Bibtex,
    Biber,
}
impl BibliographyBackend {
    pub(crate) fn executable(self) -> &'static str {
        match self {
            Self::Bibtex => "bibtex",
            Self::Biber => "biber",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct BibliographyToolConfig {
    pub backend: BibliographyBackend,
    pub executable: Option<String>,
    pub arguments: Vec<String>,
    pub dependencies: Vec<String>,
    pub timeout_seconds: u64,
}
impl Default for BibliographyToolConfig {
    fn default() -> Self {
        Self {
            backend: BibliographyBackend::Bibtex,
            executable: None,
            arguments: vec![],
            dependencies: vec![],
            timeout_seconds: 60,
        }
    }
}
impl BibliographyToolConfig {
    pub(crate) fn validate(&self) -> Result<(), String> {
        if !(1..=3600).contains(&self.timeout_seconds)
            || self
                .executable
                .as_ref()
                .is_some_and(|name| name.trim().is_empty() || name.contains('\0'))
        {
            return Err("Bibliography tools need a nonempty executable and timeoutSeconds between 1 and 3600".into());
        }
        if self.arguments.len() > 128
            || self.arguments.iter().any(|arg| arg.contains('\0'))
            || self.dependencies.len() > 128
            || self
                .dependencies
                .iter()
                .any(|path| path.trim().is_empty() || path.contains('\0'))
        {
            return Err("Bibliography tools support at most 128 literal arguments and 128 nonempty dependency paths, without NUL characters".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct IndexToolConfig {
    pub executable: String,
    pub arguments: Vec<String>,
    pub input_extension: String,
    pub output_extension: String,
    pub job_name: Option<String>,
    pub dependencies: Vec<String>,
    pub timeout_seconds: u64,
}
impl Default for IndexToolConfig {
    fn default() -> Self {
        Self {
            executable: "makeindex".into(),
            arguments: vec![
                "-o".into(),
                "%t".into(),
                "-t".into(),
                "%l".into(),
                "%i".into(),
            ],
            input_extension: "idx".into(),
            output_extension: "ind".into(),
            job_name: None,
            dependencies: vec![],
            timeout_seconds: 30,
        }
    }
}
fn valid_job_name(name: &str) -> bool {
    !name.trim().is_empty()
        && name.trim() == name
        && !matches!(name, "." | "..")
        && name
            .chars()
            .all(|ch| ch.is_alphanumeric() || matches!(ch, ' ' | '-' | '_' | '.'))
}
impl IndexToolConfig {
    pub(crate) fn is_makeindex(&self) -> bool {
        Path::new(&self.executable).file_stem() == Some(OsStr::new("makeindex"))
    }
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.executable.trim().is_empty() || !(1..=3600).contains(&self.timeout_seconds) {
            return Err(
                "Index tools need a nonempty executable and timeoutSeconds between 1 and 3600"
                    .into(),
            );
        }
        if !matches!(
            (
                self.input_extension.as_str(),
                self.output_extension.as_str()
            ),
            ("idx", "ind") | ("glo", "gls") | ("acn", "acr") | ("nlo", "nls")
        ) {
            return Err(
                "Index tools support idx/ind, glo/gls, acn/acr or nlo/nls input/output extensions"
                    .into(),
            );
        }
        if !self
            .arguments
            .iter()
            .any(|argument| argument.contains("%i"))
            || !self
                .arguments
                .iter()
                .any(|argument| argument.contains("%t"))
        {
            return Err(
                "Index tool arguments must include %i input and %t output placeholders".into(),
            );
        }
        if self.is_makeindex()
            && !self
                .arguments
                .iter()
                .any(|argument| argument.contains("%l"))
        {
            return Err(
                "makeindex arguments must include %l for a verified transcript (for example -t %l)"
                    .into(),
            );
        }
        if self
            .job_name
            .as_deref()
            .is_some_and(|name| !valid_job_name(name))
        {
            return Err("Index tool jobName must use letters, digits, spaces, '.', '_' or '-', without leading or trailing spaces or a directory path".into());
        }
        if self.dependencies.len() > 128
            || self
                .dependencies
                .iter()
                .any(|path| path.trim().is_empty() || path.contains('\0'))
        {
            return Err(
                "Index tool dependencies must contain at most 128 nonempty file paths".into(),
            );
        }
        Ok(())
    }
}

impl Config {
    pub fn parse(value: serde_json::Value) -> Result<Self, String> {
        let value = value.get("latex").unwrap_or(&value);
        let config: Self = serde_json::from_value(value.clone())
            .map_err(|error| format!("Invalid LaTeX configuration: {error}"))?;
        if !(1..=100_000).contains(&config.indexing.max_files)
            || !(1..=64 * 1024 * 1024).contains(&config.indexing.max_file_size)
            || !(1..=1024 * 1024 * 1024).contains(&config.indexing.max_total_bytes)
            || !(1..=128).contains(&config.indexing.max_depth)
            || !(1..=10_000).contains(&config.indexing.completion_limit)
        {
            return Err("Invalid indexing limits: maxFiles 1..100000, maxFileSize 1..64 MiB, maxTotalBytes 1..1 GiB, maxDepth 1..128, completionLimit 1..10000".into());
        }
        if config.diagnostics.chktex.executable.trim().is_empty()
            || !(1..=3600).contains(&config.diagnostics.chktex.timeout_seconds)
        {
            return Err(
                "ChkTeX needs a nonempty executable and timeoutSeconds between 1 and 3600".into(),
            );
        }
        if config.distribution.executable.trim().is_empty() {
            return Err("Distribution executable must not be empty".into());
        }
        if config
            .formatting
            .executable
            .as_ref()
            .is_some_and(|value| value.trim().is_empty())
        {
            return Err("Formatting executable must not be empty".into());
        }
        if !(1..=3600).contains(&config.formatting.timeout_seconds) {
            return Err("Formatting timeoutSeconds must be between 1 and 3600".into());
        }
        if config.build.executable.trim().is_empty()
            || [
                &config.build.output_directory,
                &config.build.auxiliary_directory,
                &config.build.final_output_directory,
            ]
            .into_iter()
            .flatten()
            .any(|directory| directory.trim().is_empty() || directory.contains(['\0', '\n', '\r']))
        {
            return Err("Build executable and outputDirectory must not be empty".into());
        }
        if config.build.arguments.is_none()
            && Path::new(&config.build.executable).file_stem() != Some(OsStr::new("latexmk"))
            && (config.build.auxiliary_directory.is_some()
                || config.build.final_output_directory.is_some())
        {
            return Err("Separate auxiliaryDirectory/finalOutputDirectory require latexmk or custom build arguments".into());
        }
        if !(1..=3600).contains(&config.build.timeout_seconds) {
            return Err("Build timeoutSeconds must be between 1 and 3600".into());
        }
        if let Some(name) = &config.build.job_name {
            if !valid_job_name(name) {
                return Err("Build jobName must use letters, digits, spaces, '.', '_' or '-', without leading or trailing spaces or a directory path".into());
            }
            if config.build.arguments.is_none()
                && Path::new(&config.build.executable).file_stem() == Some(OsStr::new("tectonic"))
            {
                return Err("Tectonic does not support jobName with its default arguments".into());
            }
        }
        if config.build.index_tools.len() > 32 {
            return Err("At most 32 index tools may be configured".into());
        }
        let mut outputs = std::collections::BTreeSet::new();
        for tool in &config.build.index_tools {
            tool.validate()?;
            if !outputs.insert((&tool.job_name, &tool.output_extension)) {
                return Err("Multiple index tools write the same output; choose different jobName or outputExtension".into());
            }
        }
        if let Some(tool) = &config.build.bibliography_tool {
            tool.validate()?;
        }
        if !config.build.index_tools.is_empty() || config.build.bibliography_tool.is_some() {
            config.build.rebuild_arguments()?;
        }
        Ok(config)
    }
}

impl BuildConfig {
    /// Resolve once per build so compiler arguments, notifications and artifact
    /// lookup share the same settings if the rc file changes while the tool runs.
    pub(crate) fn resolved(&self, root: &Path) -> Self {
        if self.resolution.native {
            return self.clone();
        }
        if let Some(config) = latexmk_native::cached(root, self) {
            return config;
        }
        let mut config = self.clone();
        if config.output_directory.is_some()
            && config.engine.is_some()
            && config.auxiliary_directory.is_some()
            && config.final_output_directory.is_some()
        {
            return config;
        }
        let mut settings = latexmk::Settings::default();
        let mut selected = None;
        let paths = if self.arguments.is_none()
            && !(self.resolution.rebuild && self.rebuild_arguments.is_some())
            && Path::new(&self.executable).file_stem() == Some(OsStr::new("latexmk"))
        {
            latexmk_native::candidates(root)
        } else {
            vec![]
        };
        for path in paths {
            if !crate::diagnostics::diagnostic_inputs::is_file(&path) {
                continue;
            }
            let next = latexmk::read(&path);
            macro_rules! overlay {
                ($field:ident) => {
                    if !matches!(next.$field, Ok(None)) {
                        settings.$field = next.$field;
                    }
                };
            }
            overlay!(engine);
            overlay!(output_directory);
            overlay!(auxiliary_directory);
            overlay!(final_output_directory);
            if !config.resolution.sources.contains(&path) {
                config.resolution.sources.push(path.clone());
            }
            selected = Some(path);
        }
        let describe = |setting: &str, error: String| {
            format!(
                "{}: {setting}: {error}; configure the corresponding build setting explicitly",
                selected
                    .as_ref()
                    .map_or_else(|| "latexmkrc".into(), |path| path.display().to_string())
            )
        };
        if config.engine.is_none() {
            match settings.engine {
                Ok(engine) => {
                    config.engine = engine;
                    config.resolution.inferred.push("engine".into());
                }
                Err(error) => config.resolution.issues.push(describe("$pdf_mode", error)),
            }
        }
        if config.output_directory.is_none() {
            match settings.output_directory {
                Ok(directory) => {
                    config.output_directory =
                        Some(directory.unwrap_or_else(|| DEFAULT_OUTPUT_DIRECTORY.into()));
                    config.resolution.inferred.push("outputDirectory".into());
                }
                Err(error) => config.resolution.issues.push(describe("$out_dir", error)),
            }
        }
        for (target, setting, value) in [
            (
                &mut config.auxiliary_directory,
                "$aux_dir",
                settings.auxiliary_directory,
            ),
            (
                &mut config.final_output_directory,
                "$out2_dir",
                settings.final_output_directory,
            ),
        ] {
            if target.is_none() {
                match value {
                    Ok(value) => {
                        *target = value
                            .filter(|value| !value.is_empty())
                            .or_else(|| config.output_directory.clone());
                        config.resolution.inferred.push(
                            if setting == "$aux_dir" {
                                "auxiliaryDirectory"
                            } else {
                                "finalOutputDirectory"
                            }
                            .into(),
                        );
                    }
                    Err(error) => config.resolution.issues.push(describe(setting, error)),
                }
            }
        }
        if self.arguments.is_some() || (self.resolution.rebuild && self.rebuild_arguments.is_some())
        {
            config.resolution.sources = config
                .latexmkrc_paths(root)
                .into_iter()
                .filter(|path| crate::diagnostics::diagnostic_inputs::is_file(path))
                .collect();
        }
        config.resolution.issues.sort();
        config.resolution.issues.dedup();
        config
    }

    pub(crate) fn unresolved(&self) -> Self {
        let mut config = self.clone();
        for field in &self.resolution.inferred {
            match field.as_str() {
                "engine" => config.engine = None,
                "outputDirectory" => config.output_directory = None,
                "auxiliaryDirectory" => config.auxiliary_directory = None,
                "finalOutputDirectory" => config.final_output_directory = None,
                _ => {}
            }
        }
        config.resolution = BuildResolution {
            rebuild: self.resolution.rebuild,
            ..Default::default()
        };
        config
    }

    pub(crate) fn resolve_for_build(
        &self,
        root: &Path,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> Result<Self, String> {
        if self.resolution.native {
            return Ok(self.clone());
        }
        let config = self.resolved(root);
        if config.resolution.issues.is_empty() && !config.resolution.native {
            return Ok(config);
        }
        latexmk_native::resolve(root, &config, cancel)
    }

    pub(crate) fn validate_resolution(&self) -> Result<(), String> {
        if self.resolution.issues.is_empty() {
            Ok(())
        } else {
            Err(self.resolution.issues.join("\n"))
        }
    }

    pub(crate) fn latexmkrc_paths(&self, root: &Path) -> Vec<PathBuf> {
        if Path::new(&self.executable).file_stem() != Some(OsStr::new("latexmk")) {
            return vec![];
        }
        let arguments = if self.resolution.rebuild {
            self.rebuild_arguments.as_ref().or(self.arguments.as_ref())
        } else {
            self.arguments.as_ref()
        };
        let mut paths = if let Some(arguments) = arguments {
            // Expand templates once using the same configured paths as the
            // compiler; inserted percent sequences remain literal.
            let output = root.parent().unwrap_or_else(|| Path::new(".")).join(
                self.output_directory
                    .as_deref()
                    .unwrap_or(DEFAULT_OUTPUT_DIRECTORY),
            );
            let job = self.job_name(root).unwrap_or_default().to_string_lossy();
            let expanded: Vec<_> = arguments
                .iter()
                .map(|arg| {
                    expand_template(
                        arg,
                        &root.to_string_lossy(),
                        &output.to_string_lossy(),
                        &job,
                    )
                })
                .collect();
            latexmk_native::argument_dependencies(root, &expanded)
        } else {
            latexmk_native::candidates(root)
        };
        paths.extend(self.resolution.dependencies.clone());
        paths.sort();
        paths.dedup();
        paths
    }

    pub(crate) fn for_build_mode(&self, root: &Path, rebuild: bool) -> Self {
        if self.rebuild_arguments.is_some() && self.resolution.rebuild != rebuild {
            let mut config = self.unresolved();
            config.resolution.rebuild = rebuild;
            return config.resolved(root);
        }
        self.clone()
    }

    pub fn output_path(&self, root: &Path) -> Option<PathBuf> {
        let config = self.resolved(root);
        Some(root.parent()?.join(config.output_directory.as_deref()?))
    }

    pub fn auxiliary_path(&self, root: &Path) -> Option<PathBuf> {
        Some(
            root.parent()?
                .join(self.resolved(root).auxiliary_directory.as_deref()?),
        )
    }

    pub fn final_output_path(&self, root: &Path) -> Option<PathBuf> {
        Some(
            root.parent()?
                .join(self.resolved(root).final_output_directory.as_deref()?),
        )
    }

    pub fn job_name<'a>(&'a self, root: &'a Path) -> Option<&'a OsStr> {
        self.job_name
            .as_deref()
            .map(OsStr::new)
            .or_else(|| root.file_stem())
    }

    pub fn artifact_path(&self, root: &Path, extension: &str) -> Option<PathBuf> {
        let mut name = self.job_name(root)?.to_os_string();
        name.push(".");
        name.push(extension);
        let directory = match extension {
            "pdf" | "synctex" | "synctex.gz" | "ps" | "hnt" => self.final_output_path(root)?,
            "dvi" | "xdv" => self.output_path(root)?,
            _ => self.auxiliary_path(root)?,
        };
        Some(directory.join(name))
    }

    /// Substitute only template placeholders, never text inserted by another
    /// placeholder (a real source directory can itself contain `%o` or `%j`).
    pub fn expanded_arguments(&self, root: &Path) -> Option<Vec<String>> {
        let config = self.resolved(root);
        config.expand_arguments(root, config.arguments(), config.arguments.is_none())
    }

    pub fn expanded_rebuild_arguments(&self, root: &Path) -> Result<Vec<String>, String> {
        let config = self.resolved(root);
        config.validate_resolution()?;
        config
            .expand_arguments(
                root,
                config.rebuild_arguments()?,
                config.rebuild_arguments.is_none() && config.arguments.is_none(),
            )
            .ok_or_else(|| "Invalid root filename".into())
    }

    fn expand_arguments(
        &self,
        root: &Path,
        arguments: Vec<String>,
        default_arguments: bool,
    ) -> Option<Vec<String>> {
        let output = self.output_path(root)?;
        let root_name = root.to_string_lossy();
        let output_name = output.to_string_lossy();
        let job_name = self.job_name(root)?.to_string_lossy();
        Some(
            arguments
                .into_iter()
                .map(|argument| {
                    if default_arguments
                        && [
                            self.auxiliary_directory
                                .as_ref()
                                .map(|directory| format!("-auxdir={directory}")),
                            self.final_output_directory
                                .as_ref()
                                .map(|directory| format!("-out2dir={directory}")),
                        ]
                        .into_iter()
                        .flatten()
                        .any(|literal| literal == argument)
                    {
                        argument
                    } else {
                        expand_template(&argument, &root_name, &output_name, &job_name)
                    }
                })
                .collect(),
        )
    }

    pub fn effective_engine(&self) -> Option<Engine> {
        if self.arguments.is_some() || !self.resolution.issues.is_empty() {
            return None;
        }
        let executable = std::path::Path::new(&self.executable)
            .file_stem()?
            .to_str()?;
        if executable == "latexmk" {
            Some(self.engine.unwrap_or(Engine::PdfLatex))
        } else {
            Engine::from_program(executable)
        }
    }

    /// Explicit backends process their own type. Otherwise only the known
    /// default driver promises a bibliography cycle with arbitrary requests.
    pub(crate) fn rebuilds_bibliography(&self, required: &str) -> bool {
        if let Some(tool) = &self.bibliography_tool {
            return tool.validate().is_ok() && tool.backend.executable() == required;
        }
        self.arguments.is_none()
            && self.rebuild_arguments.is_none()
            && Path::new(&self.executable).file_stem() == Some(OsStr::new("latexmk"))
    }

    pub fn rebuild_arguments(&self) -> Result<Vec<String>, String> {
        if let Some(arguments) = &self.rebuild_arguments {
            return Ok(arguments.clone());
        }
        if self.arguments.is_none() {
            let executable = Path::new(&self.executable)
                .file_stem()
                .and_then(|name| name.to_str());
            let mut arguments = self.arguments();
            match executable {
                Some("latexmk") => arguments.insert(0, "-g".into()),
                // These compilers run the document each time they are invoked.
                Some("pdflatex" | "xelatex" | "lualatex" | "tectonic") => {}
                _ => {
                    return Err("Configure build.rebuildArguments for custom build commands".into())
                }
            }
            return Ok(arguments);
        }
        Err("Configure build.rebuildArguments for custom build commands".into())
    }

    pub fn arguments(&self) -> Vec<String> {
        if let Some(arguments) = &self.arguments {
            return arguments.clone();
        }
        let engine = std::path::Path::new(&self.executable)
            .file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        let arguments: &[&str] = match engine {
            "tectonic" => &[
                "-X",
                "compile",
                "--synctex",
                "--keep-logs",
                "--outdir",
                "%o",
                "%f",
            ],
            "pdflatex" | "xelatex" | "lualatex" => &[
                "-recorder",
                "-interaction=nonstopmode",
                "-file-line-error",
                "-synctex=1",
                "-no-shell-escape",
                "-output-directory=%o",
                "%f",
            ],
            _ => &[
                "-pdf",
                "-nobibfudge",
                "-interaction=nonstopmode",
                "-file-line-error",
                "-synctex=1",
                "-no-shell-escape",
                "-outdir=%o",
                "%f",
            ],
        };
        let mut arguments: Vec<String> = arguments
            .iter()
            .map(|argument| (*argument).into())
            .collect();
        if self.job_name.is_some() && engine != "tectonic" {
            arguments.insert(0, "-jobname=%j".into());
        }
        if let Some(engine) = self.engine {
            if let Some(argument) = arguments.iter_mut().find(|argument| *argument == "-pdf") {
                *argument = engine.latexmk_argument().into();
            }
        }
        if engine == "latexmk" {
            for (flag, directory) in [
                ("-auxdir", &self.auxiliary_directory),
                ("-out2dir", &self.final_output_directory),
            ] {
                if let Some(directory) = directory
                    .as_ref()
                    .filter(|directory| Some(*directory) != self.output_directory.as_ref())
                {
                    arguments.insert(0, format!("{flag}={directory}"));
                }
            }
        }
        // BibTeX's default file policy rejects hidden or parent-relative output
        // paths. Keep latexmk's directory workaround for explicitly configured
        // paths that need it; ordinary in-project paths can preserve source cwd.
        let output = Path::new(
            self.auxiliary_directory
                .as_deref()
                .or(self.output_directory.as_deref())
                .unwrap_or(DEFAULT_OUTPUT_DIRECTORY),
        );
        let needs_fudge = output.is_absolute() || output.components().any(|component| {
            matches!(component, std::path::Component::ParentDir)
                || matches!(component, std::path::Component::Normal(name) if name.to_str().is_some_and(|name| name.starts_with('.')))
        });
        if needs_fudge {
            for argument in &mut arguments {
                if argument == "-nobibfudge" {
                    *argument = "-bibfudge".into();
                }
            }
        }
        arguments
    }
}

fn expand_template(argument: &str, root_name: &str, output_name: &str, job_name: &str) -> String {
    let mut expanded = String::new();
    let mut tail = argument;
    while let Some(index) = tail.find('%') {
        expanded.push_str(&tail[..index]);
        let value = match tail.as_bytes().get(index + 1) {
            Some(b'f') => Some(root_name),
            Some(b'o') => Some(output_name),
            Some(b'j') => Some(job_name),
            _ => None,
        };
        if let Some(value) = value {
            expanded.push_str(value);
            tail = &tail[index + 2..];
        } else {
            expanded.push('%');
            tail = &tail[index + 1..];
        }
    }
    expanded.push_str(tail);
    expanded
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rebuild_defaults_force_latexmk_and_preserve_engine_and_job_arguments() {
        for engine in ["pdflatex", "xelatex", "lualatex"] {
            let config = Config::parse(json!({"build":{"executable":"/tools/latexmk","engine":engine,"jobName":"draft.v2"}})).unwrap().build;
            let normal = config.arguments();
            let forced = config.rebuild_arguments().unwrap();
            assert_eq!(forced[0], "-g");
            assert_eq!(&forced[1..], normal);
            assert!(config
                .expanded_rebuild_arguments(Path::new("/tmp/main.tex"))
                .unwrap()
                .contains(&"-jobname=draft.v2".into()));
            assert!(!normal.contains(&"-g".into()));
        }
        for executable in ["pdflatex", "xelatex", "lualatex", "tectonic"] {
            let config = BuildConfig {
                executable: executable.into(),
                ..Default::default()
            };
            assert_eq!(config.rebuild_arguments().unwrap(), config.arguments());
        }
    }

    #[test]
    fn custom_rebuild_arguments_are_explicit_and_expand_placeholders_once() {
        for executable in ["latexmk", "pdflatex", "custom"] {
            let config = BuildConfig {
                executable: executable.into(),
                arguments: Some(vec!["normal".into()]),
                ..Default::default()
            };
            assert!(config
                .rebuild_arguments()
                .unwrap_err()
                .contains("build.rebuildArguments"));
        }
        assert!(BuildConfig {
            executable: "custom".into(),
            ..Default::default()
        }
        .rebuild_arguments()
        .is_err());
        let config = Config::parse(json!({"build":{"executable":"custom","arguments":["normal"],"rebuildArguments":["force;$(false)","%f","%o","%j","%unknown%%"],"outputDirectory":"out%f"}})).unwrap().build;
        assert_eq!(config.arguments(), vec!["normal"]);
        assert_eq!(
            config
                .expanded_rebuild_arguments(Path::new("/tmp/%o project/main%j.tex"))
                .unwrap(),
            vec![
                "force;$(false)",
                "/tmp/%o project/main%j.tex",
                "/tmp/%o project/out%f",
                "main%j",
                "%unknown%%"
            ]
        );
    }

    #[test]
    fn job_names_use_consistent_artifact_paths_and_validate_supported_tool_arguments() {
        let root = Path::new("/tmp/project/main.tex");
        for executable in ["latexmk", "pdflatex", "xelatex", "lualatex"] {
            let config =
                Config::parse(json!({"build":{"executable":executable,"jobName":"论文 draft.v2"}}))
                    .unwrap()
                    .build;
            assert!(config
                .expanded_arguments(root)
                .unwrap()
                .contains(&"-jobname=论文 draft.v2".into()));
            assert_eq!(
                config.artifact_path(root, "pdf").unwrap(),
                Path::new("/tmp/project/build/texquill/论文 draft.v2.pdf")
            );
            assert_eq!(
                config.artifact_path(root, "synctex.gz").unwrap(),
                Path::new("/tmp/project/build/texquill/论文 draft.v2.synctex.gz")
            );
        }
        for name in [
            "",
            " ",
            ".",
            "..",
            "draft ",
            "../draft",
            "a/b",
            "a\\b",
            "a\nb",
            "a%o",
            "a;command",
            "`command`",
            "a'b",
            "a\"b",
        ] {
            assert!(
                Config::parse(json!({"build":{"jobName":name}})).is_err(),
                "{name}"
            );
        }
        assert!(
            Config::parse(json!({"build":{"executable":"tectonic","jobName":"draft"}})).is_err()
        );
        let custom = Config::parse(json!({"build":{"executable":"custom","jobName":"draft","arguments":["--output=%o/%j.pdf", "%f"]}})).unwrap().build;
        assert_eq!(custom.arguments(), vec!["--output=%o/%j.pdf", "%f"]);
        assert_eq!(
            custom.expanded_arguments(root).unwrap(),
            vec![
                "--output=/tmp/project/build/texquill/draft.pdf",
                "/tmp/project/main.tex"
            ]
        );
    }

    #[test]
    fn argument_placeholders_do_not_expand_percent_sequences_inside_source_paths() {
        let root = Path::new("/tmp/%o project/main%j.tex");
        let config = BuildConfig {
            output_directory: Some("out%f".into()),
            arguments: Some(vec![
                "%f".into(),
                "%o".into(),
                "%j".into(),
                "%unknown%%".into(),
            ]),
            ..Default::default()
        };
        assert_eq!(
            config.expanded_arguments(root).unwrap(),
            vec![
                "/tmp/%o project/main%j.tex",
                "/tmp/%o project/out%f",
                "main%j",
                "%unknown%%"
            ]
        );
    }

    #[test]
    fn validates_configuration_and_engine_defaults() {
        assert!(!Config::default().inlay_hints.pages);
        assert!(
            !Config::parse(json!({"inlayHints":{"numbers":true}}))
                .unwrap()
                .inlay_hints
                .pages
        );
        assert!(
            Config::parse(json!({"inlayHints":{"pages":true}}))
                .unwrap()
                .inlay_hints
                .pages
        );
        let config =
            Config::parse(json!({"latex":{"build":{"executable":"pdflatex","onSave":true}}}))
                .unwrap();
        assert!(config.build.on_save);
        assert!(config
            .build
            .arguments()
            .contains(&"-output-directory=%o".into()));
        assert!(!config.build.arguments().contains(&"-pdf".into()));
        assert!(Config::parse(json!({"build":{"timeoutSeconds":0}})).is_err());
        assert!(Config::parse(json!({"build":{"timeoutSecond":30}})).is_err());
        assert!(Config::parse(json!({"build":{"executable":""}})).is_err());
        assert!(BuildConfig::default()
            .arguments()
            .contains(&"-nobibfudge".into()));
        let config = BuildConfig {
            output_directory: Some(".texquill".into()),
            ..Default::default()
        };
        assert!(config.arguments().contains(&"-bibfudge".into()));
        let config = BuildConfig {
            output_directory: Some("../output".into()),
            ..Default::default()
        };
        assert!(config.arguments().contains(&"-bibfudge".into()));
    }

    #[test]
    fn latexmk_engine_configuration_preserves_explicit_executables_and_arguments() {
        for (engine, flag) in [
            ("pdflatex", "-pdf"),
            ("xelatex", "-xelatex"),
            ("lualatex", "-lualatex"),
        ] {
            let config = Config::parse(json!({"build":{"engine":engine}})).unwrap();
            assert!(config.build.arguments().contains(&flag.into()));
            assert_eq!(
                serde_json::to_value(config.build.effective_engine()).unwrap(),
                json!(engine)
            );
        }
        let config =
            Config::parse(json!({"build":{"executable":"pdflatex","engine":"xelatex"}})).unwrap();
        assert_eq!(config.build.effective_engine(), Some(Engine::PdfLatex));
        assert!(!config.build.arguments().contains(&"-xelatex".into()));
        let config =
            Config::parse(json!({"build":{"engine":"xelatex","arguments":["custom", "%f"]}}))
                .unwrap();
        assert_eq!(config.build.arguments(), vec!["custom", "%f"]);
        assert_eq!(config.build.effective_engine(), None);
        assert!(Config::parse(json!({"build":{"engine":"bash"}})).is_err());
    }
}
