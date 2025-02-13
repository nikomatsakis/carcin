use std::ffi::OsString;
use std::io::Write;
use std::path::Path;
use std::process::{ChildStdin, Command, ExitStatus, Stdio};

use anyhow::Context;
use cargo_metadata::camino::Utf8PathBuf;
use clap::Parser;
use serde::{Deserialize, Serialize};

/// Main function for the gluegun CLI.
pub fn cli_main() -> anyhow::Result<()> {
    Builder::from_env()?.execute()
}

/// Struct to customize GlueGun CLI execution.
pub struct Builder {
    current_directory: Utf8PathBuf,
    args: Vec<OsString>,
    plugin_command: Box<dyn Fn(
        &serde_json::Value,
        &str,
    ) -> anyhow::Result<Command>>,
}

impl Builder {
    /// Create builder with given directory and arguments.
    /// Note that `args` should begin with the command name (like `argv[0]` in C).
    pub fn new(
        current_directory: impl AsRef<Path>,
        args: impl IntoIterator<Item = impl Into<OsString> + Clone>,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            current_directory: Utf8PathBuf::try_from(current_directory.as_ref().to_path_buf())?,
            args: args.into_iter().map(Into::into).collect(),
            plugin_command: Box::new(Self::default_plugin_command),
        })
    }

    /// Create builder with data from current environment.
    pub fn from_env() -> anyhow::Result<Self> {
        Self::new(std::env::current_dir()?, std::env::args_os())
    }

    /// Customize the code to create the plugin command
    /// 
    /// The function will be invoked with the workspace/package `metadata.gluegun` field
    /// along with the name of the plugin. It should return a new `Command` object.
    pub fn plugin_command(mut self, 
        plugin_command: impl Fn(
            &serde_json::Value,
            &str,
        ) -> anyhow::Result<Command> + 'static,
    ) -> Self {
        self.plugin_command = Box::new(plugin_command);
        self
    }

    /// Execute cargo-gluegun.
    pub fn execute(self) -> anyhow::Result<()> {
        let cli = Cli::try_parse_from(&self.args)?;

        let metadata = cli
            .manifest
            .metadata()
            .current_dir(&self.current_directory)
            .exec()?;
        let (selected, _excluded) = cli.workspace.partition_packages(&metadata);

        if selected.is_empty() {
            anyhow::bail!("no packages selected -- you may have misspelled the package name?");
        }

        if cli.plugins.is_empty() {
            anyhow::bail!("no plugins specified");
        }

        for package in selected {
            for plugin in &cli.plugins {
                self.apply_plugin(plugin, &metadata.workspace_metadata, package)?;
            }
        }

        Ok(())
    }

    fn apply_plugin(
        &self,
        plugin: &str,
        workspace_metadata: &serde_json::Value,
        package: &cargo_metadata::Package,
    ) -> anyhow::Result<()> {
        if let Some(_) = package.source {
            anyhow::bail!("{pkg}: can only process local packages", pkg = package.name);
        }

        // FIXME: Don't be so hacky. My god Niko, you should be ashamed of yourself.
        let cargo_toml_path = &package.manifest_path;
        let manifest_dir = cargo_toml_path.parent().unwrap();
        let src_lib_rs = manifest_dir.join("src/lib.rs");

        let idl = gluegun_idl::Parser::new()
            .parse_crate_named(&package.name, &manifest_dir, &src_lib_rs)
            .with_context(|| format!("extracting interface from `{src_lib_rs}`"))?;

        // Extract gluegun metadata (if any).
        let gluegun_workspace_metadata = workspace_metadata.get("gluegun");
        let gluegun_package_metadata = package.metadata.get("gluegun");
        let gluegun_metadata = merge_metadata(gluegun_workspace_metadata, gluegun_package_metadata)
            .with_context(|| format!("merging workspace and package metadata"))?;

        // Search for `workspace.metadata.gluegun.tool_name` and
        // `package.metadata.gluegun.tool_name`.
        let plugin_workspace_metadata = gluegun_workspace_metadata.and_then(|v| v.get(plugin));
        let plugin_package_metadata = gluegun_package_metadata.and_then(|v| v.get(plugin));
        let plugin_metadata = merge_metadata(plugin_workspace_metadata, plugin_package_metadata)
            .with_context(|| format!("merging workspace and package metadata"))?;

        // Compute destination crate name and path
        let (crate_name, crate_path) =
            dest_crate_name_and_path(plugin, &gluegun_metadata, package)
                .with_context(|| format!("computing destination crate name and path"))?;

        // Execute the plugin
        let exit_status = self
            .execute_plugin(
                plugin,
                &gluegun_metadata,
                &idl,
                &plugin_metadata,
                &crate_name,
                &crate_path,
            )
            .with_context(|| format!("executing plugin `{plugin}`"))?;

        if exit_status.success() {
            Ok(())
        } else {
            anyhow::bail!("gluegun-{plugin} failed with code {exit_status}");
        }
    }

    fn execute_plugin(
        &self,
        plugin: &str,
        gluegun_metadata: &serde_json::Value,
        idl: &gluegun_idl::Idl,
        metadata: &serde_json::Value,
        crate_name: &str,
        crate_path: &Utf8PathBuf,
    ) -> anyhow::Result<ExitStatus> {
        // Create the plugin command using the hook supplied by configuration.
        // Default is to run `Self::default_plugin_command` below.
        let mut plugin_command = (self.plugin_command)(
            gluegun_metadata,
            plugin,
        ).with_context(|| format!("creating plugin command"))?;

        // Configure the command.
        plugin_command
            .current_dir(&self.current_directory)
            .arg(format!("gg-{}", plugin))
            .stdin(Stdio::piped()) // Configure stdin
            .stdout(Stdio::inherit()) // Configure stdout
            .stderr(Stdio::inherit());
        

        // Execute the helper
        eprintln!("{plugin_command:?}");
        let mut child = plugin_command 
            .spawn()
            .with_context(|| format!("spawning gluegun-{plugin}"))?;

        // Write the data to the child's stdin.
        // This has to be kept in sync with the definition from `gluegun_core::cli`.
        let Some(stdin) = child.stdin.take() else {
            anyhow::bail!("failed to take stdin");
        };
        let write_data = |mut stdin: ChildStdin| -> anyhow::Result<()> {
            writeln!(stdin, r#"{{"#)?;
            writeln!(stdin, r#"  "idl": {},"#, serde_json::to_string(&idl)?)?;
            writeln!(
                stdin,
                r#"  "metadata": {},"#,
                serde_json::to_string(&metadata)?
            )?;
            writeln!(stdin, r#"  "dest_crate": {{"#)?;
            writeln!(stdin, r#"    "crate_name": {crate_name:?},"#)?;
            writeln!(stdin, r#"    "path": {crate_path:?}"#)?;
            writeln!(stdin, r#"  }}"#)?;
            writeln!(stdin, r#"}}"#)?;
            Ok(())
        };
        write_data(stdin).with_context(|| format!("writing data to gluegun-{plugin}"))?;
        eprintln!("output data successful");

        Ok(child
            .wait()
            .with_context(|| format!("waiting for gluegun-{plugin}"))?)
    }

    fn default_plugin_command(
        gluegun_metadata: &serde_json::Value,
        plugin: &str,
    ) -> anyhow::Result<Command> {
        if let Some(c) =
            Self::customized_plugin_command(gluegun_metadata, plugin)?
        {
            return Ok(c);
        }

        Ok(Command::new(format!("gluegun-{plugin}")))
    }

    fn customized_plugin_command(
        gluegun_metadata: &serde_json::Value,
        plugin: &str,
    ) -> anyhow::Result<Option<Command>> {
        let Some(plugin_command) = gluegun_metadata.get("plugin-command") else {
            return Ok(None);
        };

        let serde_json::Value::String(plugin_command) = plugin_command else {
            anyhow::bail!("expected a string for workspace configuration `gluegun.plugin_command`")
        };

        // should probably...do something better...
        let s = plugin_command.replace("{plugin}", plugin);
        if s.contains("'") {
            anyhow::bail!("`gluegun.plugin_command` cannot contain `'` characters (FIXME)")
        }

        let mut words = s.split_whitespace();
        let Some(word0) = words.next() else {
            anyhow::bail!("expected at least one word in `gluegun.plugin_command`")
        };

        let mut cmd = Command::new(word0);
        cmd.args(words);

        Ok(Some(cmd))
    }
}

/// A simple Cli you can use for your own parser.
#[derive(clap::Parser)]
struct Cli {
    #[command(flatten)]
    manifest: clap_cargo::Manifest,

    #[command(flatten)]
    workspace: clap_cargo::Workspace,

    /// Specify a list of plugins to use.
    plugins: Vec<String>,
}

fn dest_crate_name_and_path(
    plugin: &str,
    gluegun_metadata: &serde_json::Value,
    package: &cargo_metadata::Package,
) -> anyhow::Result<(String, Utf8PathBuf)> {
    // Find the configuration (if any)
    let dp: DestinationPath = gluegun_metadata.get("destination-path").and_then(|v| Some(serde_json::from_value(v.clone()))).unwrap_or(Ok(DestinationPath::Child))?;

    // Default crate name is `foo-x`, taken from the plugin
    let crate_name = format!("{}-{plugin}", package.name);

    // Parent directory: either the directory containing the
    // `Cargo.toml` (child of target crate) or the parent of that
    // directory (sibling of target crate), based on the configuration.
    let package_parent = match dp {
        DestinationPath::Child => package.manifest_path.parent(),
        DestinationPath::Sibling => package.manifest_path.parent().and_then(|p| p.parent()),
    };
    
    // Directory must exist or we get an error
    let Some(package_parent) = package_parent else {
        anyhow::bail!(
            "cannot compute parent path for crate at `{}`",
            package.manifest_path
        );
    };
    let crate_path = package_parent.join(&crate_name);

    Ok((crate_name, crate_path))
}

/// Merge metadata from workspace/package
fn merge_metadata(
    workspace_metadata: Option<&serde_json::Value>,
    package_metadata: Option<&serde_json::Value>,
) -> anyhow::Result<serde_json::Value> {
    match (workspace_metadata, package_metadata) {
        (Some(workspace), Some(package)) => merge_values(workspace, package),
        (Some(workspace), None) => Ok(workspace.clone()),
        (None, Some(package)) => Ok(package.clone()),
        (None, None) => Ok(serde_json::Value::Null),
    }
}

/// Merge metadata values from workspace/package.
///
/// Generally speaking, package wins, but for maps we take the keys from workspace that are not present in package.
fn merge_values(
    workspace_value: &serde_json::Value,
    package_value: &serde_json::Value,
) -> anyhow::Result<serde_json::Value> {
    match (workspace_value, package_value) {
        (serde_json::Value::Null, serde_json::Value::Null) => Ok(serde_json::Value::Null),

        (serde_json::Value::Number(_), serde_json::Value::Number(_))
        | (serde_json::Value::Bool(_), serde_json::Value::Bool(_))
        | (serde_json::Value::String(_), serde_json::Value::String(_))
        | (serde_json::Value::Array(_), serde_json::Value::Array(_)) => Ok(package_value.clone()),

        (serde_json::Value::Object(workspace_map), serde_json::Value::Object(package_map)) => {
            let mut merged = workspace_map.clone();

            for (key, value) in package_map {
                merged.insert(key.clone(), value.clone());
            }

            Ok(serde_json::Value::Object(merged))
        }

        (serde_json::Value::Null, _)
        | (serde_json::Value::Number(_), _)
        | (serde_json::Value::Bool(_), _)
        | (serde_json::Value::String(_), _)
        | (serde_json::Value::Array(_), _)
        | (_, serde_json::Value::Null)
        | (_, serde_json::Value::Number(_))
        | (_, serde_json::Value::Bool(_))
        | (_, serde_json::Value::String(_))
        | (_, serde_json::Value::Array(_)) => anyhow::bail!(
            "cannot merge workspace/package configuration:\
            \n    workspace: {workspace_value}\
            \n    package: {package_value}"
        ),
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum DestinationPath {
    Child,
    Sibling,
}