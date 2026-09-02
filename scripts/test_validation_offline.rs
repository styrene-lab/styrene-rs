use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    std::env::current_dir().expect("resolve repository root")
}

fn read(path: impl AsRef<Path>) -> String {
    let path = path.as_ref();
    fs::read_to_string(root().join(path))
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
}

fn walk_files(path: &Path, extensions: &[&str], output: &mut Vec<PathBuf>) {
    for entry in
        fs::read_dir(path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
    {
        let entry = entry.expect("read directory entry");
        let file_type = entry.file_type().expect("read file type");
        let path = entry.path();
        if file_type.is_dir() {
            walk_files(&path, extensions, output);
        } else if file_type.is_file()
            && path.extension().and_then(|value| value.to_str()).is_some_and(|value| {
                extensions.iter().any(|extension| value.eq_ignore_ascii_case(extension))
            })
        {
            output.push(path);
        }
    }
}

fn repository_files(relative: &str, extensions: &[&str]) -> Vec<PathBuf> {
    let mut files = Vec::new();
    walk_files(&root().join(relative), extensions, &mut files);
    files.sort();
    files
}

#[derive(Debug)]
struct Recipe {
    dependencies: Vec<String>,
    body: String,
}

fn recipes(justfile: &str) -> HashMap<String, Recipe> {
    let mut parsed = HashMap::new();
    let mut current: Option<String> = None;
    for line in justfile.lines() {
        if !line.starts_with(char::is_whitespace) && !line.starts_with('#') {
            if let Some((header, dependencies)) = line.split_once(':') {
                if !header.contains(" := ") && !header.contains(" = ") {
                    let name = header.split_whitespace().next().unwrap_or_default();
                    if !name.is_empty() {
                        current = Some(name.to_string());
                        parsed.insert(
                            name.to_string(),
                            Recipe {
                                dependencies: dependencies
                                    .split_whitespace()
                                    .map(ToOwned::to_owned)
                                    .collect(),
                                body: format!("{line}\n"),
                            },
                        );
                        continue;
                    }
                }
            }
            current = None;
        }
        if let Some(name) = &current {
            let recipe = parsed.get_mut(name).expect("current recipe exists");
            recipe.body.push_str(line);
            recipe.body.push('\n');
        }
    }
    let names = parsed.keys().cloned().collect::<HashSet<_>>();
    for recipe in parsed.values_mut() {
        for line in logical_lines(&recipe.body) {
            let tokens = shell_tokens(&line);
            if let Some(index) = tokens.iter().position(|token| token == "just") {
                if let Some(dependency) = tokens[index + 1..]
                    .iter()
                    .find(|token| !token.starts_with('-') && names.contains(*token))
                {
                    if !recipe.dependencies.contains(dependency) {
                        recipe.dependencies.push(dependency.clone());
                    }
                }
            }
        }
    }
    parsed
}

fn logical_lines(document: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    for line in document.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        current.push_str(trimmed.trim_end_matches('\\'));
        current.push(' ');
        if !trimmed.ends_with('\\') {
            lines.push(current.trim().to_string());
            current.clear();
        }
    }
    if !current.trim().is_empty() {
        lines.push(current.trim().to_string());
    }
    lines
}

fn shell_tokens(line: &str) -> Vec<String> {
    line.split_whitespace()
        .map(|token| {
            token.trim_matches(|character: char| "@\"'\\;,()".contains(character)).to_string()
        })
        .filter(|token| !token.is_empty())
        .collect()
}

#[derive(Debug)]
struct CargoInvocation {
    command: String,
    workspace: bool,
    packages: Vec<String>,
    excludes: HashSet<String>,
    lib: bool,
    tests: Vec<String>,
    bins: Vec<String>,
    all_targets: bool,
    filters: Vec<String>,
    manifest_path: Option<String>,
}

fn cargo_invocations(document: &str) -> Vec<CargoInvocation> {
    logical_lines(document)
        .into_iter()
        .filter_map(|line| {
            let tokens = shell_tokens(&line);
            let cargo = tokens.iter().position(|token| token == "cargo")?;
            let command = tokens.get(cargo + 1)?.as_str();
            if !matches!(command, "test" | "check" | "clippy") {
                return None;
            }
            let mut invocation = CargoInvocation {
                command: command.to_string(),
                workspace: false,
                packages: Vec::new(),
                excludes: HashSet::new(),
                lib: false,
                tests: Vec::new(),
                bins: Vec::new(),
                all_targets: false,
                filters: Vec::new(),
                manifest_path: None,
            };
            let mut index = cargo + 2;
            while index < tokens.len() {
                match tokens[index].as_str() {
                    "--" => break,
                    "--workspace" => invocation.workspace = true,
                    "--all-targets" => invocation.all_targets = true,
                    "--lib" => invocation.lib = true,
                    "-p" | "--package" => {
                        index += 1;
                        invocation.packages.push(tokens.get(index)?.clone());
                    }
                    "--exclude" => {
                        index += 1;
                        invocation.excludes.insert(tokens.get(index)?.clone());
                    }
                    "--test" => {
                        index += 1;
                        invocation.tests.push(tokens.get(index)?.clone());
                    }
                    "--bin" => {
                        index += 1;
                        invocation.bins.push(tokens.get(index)?.clone());
                    }
                    "--manifest-path" => {
                        index += 1;
                        invocation.manifest_path = Some(tokens.get(index)?.clone());
                    }
                    "--features" | "--target" | "--profile" | "--color" | "--jobs" | "-j" => {
                        index += 1;
                        let _ = tokens.get(index)?;
                    }
                    value if value.starts_with("--package=") => {
                        invocation.packages.push(value[10..].to_string());
                    }
                    value if value.starts_with("--exclude=") => {
                        invocation.excludes.insert(value[10..].to_string());
                    }
                    value if value.starts_with("--test=") => {
                        invocation.tests.push(value[7..].to_string());
                    }
                    value if value.starts_with("--bin=") => {
                        invocation.bins.push(value[6..].to_string());
                    }
                    value if value.starts_with("--manifest-path=") => {
                        invocation.manifest_path = Some(value[16..].to_string());
                    }
                    value if value.starts_with("-p") && value.len() > 2 => {
                        panic!("unsupported compact Cargo package selector: {value}")
                    }
                    value if !value.starts_with('-') => invocation.filters.push(value.to_string()),
                    _ => {}
                }
                index += 1;
            }
            Some(invocation)
        })
        .collect()
}

impl CargoInvocation {
    fn selects(&self, package: &str, target: &str) -> bool {
        let package_selected = (self.workspace && !self.excludes.contains(package))
            || self.packages.iter().any(|item| item == package);
        if !package_selected {
            return false;
        }
        if !self.tests.is_empty() {
            return target == "lib" && self.lib || self.tests.iter().any(|test| test == target);
        }
        if self.lib {
            return target == "lib";
        }
        true
    }

    fn selects_test(&self, package: &str, target: &str, test_name: &str) -> bool {
        self.selects(package, target)
            && (self.filters.is_empty() || self.filters.iter().any(|filter| test_name.contains(filter)))
    }
}

fn selected_test_matrix(invocations: &[CargoInvocation]) -> HashSet<String> {
    let mut selected = HashSet::new();
    for invocation in invocations.iter().filter(|invocation| invocation.command == "test") {
        for package in &invocation.packages {
            if invocation.lib {
                selected.insert(format!("{package}:lib"));
            }
            for test in &invocation.tests {
                selected.insert(format!("{package}:test/{test}"));
            }
            for bin in &invocation.bins {
                selected.insert(format!("{package}:bin/{bin}"));
            }
            if !invocation.lib && invocation.tests.is_empty() && invocation.bins.is_empty() {
                selected.insert(format!("{package}:*"));
            }
        }
    }
    selected
}

fn recipe_closure(
    name: &str,
    recipes: &HashMap<String, Recipe>,
    seen: &mut HashSet<String>,
) -> String {
    assert!(seen.insert(name.to_string()), "recipe dependency cycle at {name}");
    let recipe = recipes.get(name).unwrap_or_else(|| panic!("missing recipe {name}"));
    let mut closure = recipe.body.clone();
    for dependency in &recipe.dependencies {
        if recipes.contains_key(dependency) && !seen.contains(dependency) {
            closure.push_str(&recipe_closure(dependency, recipes, seen));
        }
    }
    closure
}

fn quoted_values(document: &str, key: &str) -> Vec<String> {
    let marker = format!("{key} = [");
    let start = document.find(&marker).unwrap_or_else(|| panic!("missing {key}")) + marker.len();
    let tail = &document[start..];
    let end = tail.find(']').unwrap_or_else(|| panic!("unterminated {key}"));
    tail[..end]
        .split('"')
        .enumerate()
        .filter_map(|(index, value)| (index % 2 == 1).then(|| value.to_string()))
        .collect()
}

fn sensitive_entries(document: &str) -> Vec<(String, String)> {
    document
        .split("[[sensitive_tests]]")
        .skip(1)
        .map(|block| {
            let value = |key: &str| {
                block
                    .lines()
                    .find_map(|line| line.trim().strip_prefix(&format!("{key} = \"")))
                    .and_then(|value| value.strip_suffix('"'))
                    .unwrap_or_else(|| panic!("sensitive test missing {key}: {block}"))
                    .to_string()
            };
            (value("path"), value("evidence"))
        })
        .collect()
}

fn table_blocks<'a>(document: &'a str, header: &str) -> Vec<&'a str> {
    document.split(header).skip(1).map(|tail| tail.split("\n[[").next().unwrap_or(tail)).collect()
}

fn rns_parity_policy_errors(
    index: &str,
    consumers: &str,
    handoff: &str,
    product: &str,
    authority_documents: &[(&str, &str)],
) -> Vec<String> {
    let mut errors = Vec::new();
    for (authority_id, revision) in [
        ("rns-1.4.2", "b48b96e61676504e0a4e527b33b9a0b4495c6872"),
        ("rns-1.5.1", "149e4151095adf098b8f53eab0c03b37169e8559"),
    ] {
        if !index.contains(&format!("\"{authority_id}\""))
            || !index.contains(&format!("\"revision\": \"{revision}\""))
        {
            errors.push(format!("missing immutable authority {authority_id}"));
        }
    }
    let authority_files = authority_documents
        .iter()
        .filter(|(_, document)| document.contains("\"authorities\""))
        .map(|(path, _)| *path)
        .collect::<Vec<_>>();
    if authority_files != ["tests/interop/fixtures/rns/index-v2.json"] {
        errors.push(format!("competing RNS fixture authority schemas: {authority_files:?}"));
    }
    for consumer in [
        "beechat-rns-corrections-wave",
        "freetak-rns-hardening-wave",
        "leviculum-rns-corpus-wave",
    ] {
        if !consumers.contains(&format!("\"change_id\": \"{consumer}\"")) {
            errors.push(format!("missing shared-loader consumer {consumer}"));
        }
    }
    if !consumers.contains("\"fixture_index\": \"tests/interop/fixtures/rns/index-v2.json\"") {
        errors.push("consumer registry does not select the shared v2 index".to_string());
    }
    for required in [
        "\"runner\": \"styrene-interop-runner\"",
        "\"authority_id\": \"rns-1.5.1\"",
        "\"authority_revision\": \"149e4151095adf098b8f53eab0c03b37169e8559\"",
        "\"registered\": false",
        "\"enabled\": false",
        "\"claim_status\": \"unevidenced\"",
    ] {
        if !handoff.contains(required) {
            errors.push(format!("live handoff lost '{required}'"));
        }
    }
    let scenario_ids = [
        "rns-1.5.1-routed-link-request-channel-resource",
        "rns-1.5.1-mixed-interface-mtu",
        "rns-1.5.1-interface-discovery-observation",
    ];
    if handoff.matches("\"state\": \"handoff_only\"").count() != scenario_ids.len() {
        errors.push("expected three handoff-only live scenarios".to_string());
    }
    for required in [
        "\"timeout_secs\": ",
        "\"max_log_bytes\": ",
        "\"max_artifacts\": ",
        "\"max_artifact_bytes\": ",
        "\"artifact_sha256_required\": true",
        "\"revision_attestation_required\": true",
        "\"cancellation\": ",
        "\"cleanup\": ",
    ] {
        if handoff.matches(required).count() != scenario_ids.len() {
            errors.push(format!("live handoff scenarios lost '{required}'"));
        }
    }
    for id in scenario_ids {
        if !handoff.contains(&format!("\"id\": \"{id}\"")) {
            errors.push(format!("missing live handoff scenario {id}"));
        }
        if product.contains(&format!("id = \"{id}\"")) {
            errors.push(format!("unverified live handoff was registered as parity gate {id}"));
        }
    }
    for task in [
        "reticulum-lxmf-nomadnet-parity:4.7",
        "reticulum-lxmf-nomadnet-parity:5.7",
        "reticulum-lxmf-nomadnet-parity:8.8",
        "reticulum-lxmf-nomadnet-parity:12.6",
    ] {
        if !handoff.contains(task) {
            errors.push(format!("live handoff lost owner {task}"));
        }
    }
    for upstream in table_blocks(product, "[[parity_upstreams]]") {
        let revision = upstream
            .lines()
            .find_map(|line| line.strip_prefix("revision = \"").and_then(|value| value.strip_suffix('"')));
        if !revision.is_some_and(|revision| {
            revision.len() == 40
                && revision.bytes().all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        }) {
            errors.push("parity upstream revision is mutable or malformed".to_string());
        }
    }
    errors
}

fn enables_hardware_by_default(manifest: &str) -> bool {
    manifest
        .lines()
        .find(|line| line.starts_with("default ="))
        .is_some_and(|line| {
            ["hardware-trng", "serial", "yubikey", "keychain"]
                .iter()
                .any(|feature| line.contains(feature))
        })
}

fn workspace_packages() -> HashMap<String, PathBuf> {
    let mut packages = HashMap::new();
    for manifest in repository_files("crates", &["toml"])
        .into_iter()
        .filter(|path| path.file_name().is_some_and(|name| name == "Cargo.toml"))
    {
        let document = fs::read_to_string(&manifest).expect("read Cargo manifest");
        let mut in_package = false;
        for line in document.lines() {
            if line.starts_with('[') {
                in_package = line == "[package]";
                continue;
            }
            if in_package {
                if let Some(name) =
                    line.strip_prefix("name = \"").and_then(|value| value.strip_suffix('"'))
                {
                    assert!(
                        packages.insert(name.to_string(), manifest.clone()).is_none(),
                        "duplicate package {name}"
                    );
                    break;
                }
            }
        }
    }
    packages
}

fn workflow_triggers(workflow: &str) -> HashSet<String> {
    let mut triggers = HashSet::new();
    let mut in_on = false;
    for line in workflow.lines() {
        if !line.starts_with(char::is_whitespace) {
            in_on = false;
            let trimmed = line.trim();
            if let Some(value) = trimmed
                .strip_prefix("on:")
                .or_else(|| trimmed.strip_prefix("\"on\":"))
                .or_else(|| trimmed.strip_prefix("'on':"))
            {
                in_on = true;
                for candidate in value
                    .trim_matches(|character: char| " []{}\"'".contains(character))
                    .split([',', ' '])
                    .filter(|candidate| !candidate.is_empty())
                {
                    let candidate = candidate
                        .split(':')
                        .next()
                        .unwrap_or(candidate)
                        .trim_matches(|character| "\"'".contains(character));
                    if !candidate.is_empty() {
                        triggers.insert(candidate.to_string());
                    }
                }
            }
            continue;
        }
        if in_on {
            let trimmed = line.trim_start();
            if line.starts_with("  ") && !line.starts_with("    ") {
                if let Some((key, _)) = trimmed.split_once(':') {
                    triggers.insert(
                        key.trim_matches(|character| "\"'".contains(character)).to_string(),
                    );
                }
            }
        }
    }
    triggers
}

fn has_trigger(workflow: &str, trigger: &str) -> bool {
    workflow_triggers(workflow).contains(trigger)
}

fn relative(path: &Path) -> String {
    path.strip_prefix(root()).expect("repository file is below root").to_string_lossy().into_owned()
}

fn delegated_paths(document: &str) -> HashSet<String> {
    let mut paths = HashSet::new();
    for line in logical_lines(document) {
        let tokens = shell_tokens(&line);
        for (index, token) in tokens.iter().enumerate() {
            let candidate = token.trim_start_matches("./");
            if (candidate.starts_with("scripts/") || token.starts_with("./"))
                && root().join(candidate).is_file()
            {
                paths.insert(candidate.to_string());
            }
            if matches!(token.as_str(), "sh" | "bash" | "python" | "python3" | "perl") {
                if let Some(argument) = tokens[index + 1..].iter().find(|argument| {
                    !argument.starts_with('-')
                        && root().join(argument.trim_start_matches("./")).is_file()
                }) {
                    paths.insert(argument.trim_start_matches("./").to_string());
                }
            }
        }
    }
    paths
}

fn interpreter(token: &str) -> bool {
    let basename = Path::new(token).file_name().and_then(|name| name.to_str()).unwrap_or(token);
    let basename = basename.to_ascii_lowercase();
    matches!(
        basename.as_str(),
        "bash"
            | "sh"
            | "zsh"
            | "fish"
            | "dash"
            | "python"
            | "python3"
            | "pypy"
            | "pypy3"
            | "perl"
            | "ruby"
            | "node"
            | "deno"
            | "bun"
            | "php"
            | "lua"
            | "pwsh"
            | "powershell"
            | "rscript"
            | "julia"
            | "tclsh"
            | "groovy"
    ) || basename.starts_with("python3.")
        || basename.starts_with("pypy3.")
}

fn inspect_offline_commands(document: &str, context: &str) -> Result<HashSet<String>, String> {
    let mut delegated = HashSet::new();
    for line in logical_lines(document) {
        if line.ends_with(':') || line.contains(": format-check") {
            continue;
        }
        for unsupported in ["&&", "||", " | ", ";", "${", "{{", "$(", "`"] {
            if line.contains(unsupported) {
                return Err(format!("{context}: unmodeled shell construct '{unsupported}': {line}"));
            }
        }
        let tokens = shell_tokens(&line);
        if tokens.iter().any(|token| interpreter(token)) {
            return Err(format!("{context}: interpreter is forbidden: {line}"));
        }
        let executable = tokens
            .iter()
            .find(|token| !token.contains('=') || token.starts_with('/') || token.starts_with("./"))
            .map(String::as_str)
            .unwrap_or("");
        if !matches!(
            executable,
            "cargo"
                | "rustfmt"
                | "rustc"
                | "mkdir"
                | "target/check-fixture-immutability"
                | "target/test-validation-offline"
        ) {
            return Err(format!("{context}: unmodeled executable or script: {line}"));
        }
        if executable == "rustc" {
            let sources = tokens
                .iter()
                .filter(|token| token.ends_with(".rs") && root().join(token.as_str()).is_file())
                .cloned()
                .collect::<Vec<_>>();
            if sources.len() != 1 {
                return Err(format!("{context}: rustc must delegate exactly one repository source: {line}"));
            }
            delegated.insert(sources[0].clone());
        }
        for token in &tokens {
            let candidate = token.trim_start_matches("./");
            if root().join(candidate).is_file()
                && ["py", "sh", "bash", "zsh", "pl", "rb", "js", "ts", "lua"]
                    .contains(&Path::new(candidate).extension().and_then(|value| value.to_str()).unwrap_or(""))
            {
                return Err(format!("{context}: delegated script is forbidden: {candidate}"));
            }
        }
    }
    Ok(delegated)
}

fn local_action_references(document: &str) -> Vec<String> {
    document
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            line.strip_prefix("uses:").or_else(|| line.strip_prefix("- uses:"))
        })
        .map(|value| value.trim().trim_matches(['\'', '"']).to_string())
        .filter(|value| value.starts_with("./"))
        .collect()
}

fn action_run_blocks(document: &str) -> Vec<String> {
    let lines = document.lines().collect::<Vec<_>>();
    let mut blocks = Vec::new();
    let mut index = 0usize;
    while index < lines.len() {
        let line = lines[index];
        let indent = line.len() - line.trim_start().len();
        let Some(value) = line.trim_start().strip_prefix("run:") else {
            index += 1;
            continue;
        };
        let value = value.trim();
        if value != "|" && value != ">" {
            blocks.push(value.trim_matches(['\'', '"']).to_string());
            index += 1;
            continue;
        }
        index += 1;
        let mut block = String::new();
        while index < lines.len() {
            let child = lines[index];
            let child_indent = child.len() - child.trim_start().len();
            if !child.trim().is_empty() && child_indent <= indent {
                break;
            }
            block.push_str(child.trim());
            block.push('\n');
            index += 1;
        }
        blocks.push(block);
    }
    blocks
}

fn inspect_local_action(action: &str, seen: &mut HashSet<String>) -> Result<(), String> {
    let action = action.trim_start_matches("./");
    if !seen.insert(action.to_string()) {
        return Ok(());
    }
    let directory = root().join(action);
    let manifest = [directory.join("action.yml"), directory.join("action.yaml")]
        .into_iter()
        .find(|candidate| candidate.is_file())
        .ok_or_else(|| format!("local action has no manifest: {action}"))?;
    let document = fs::read_to_string(&manifest).map_err(|error| format!("read {}: {error}", manifest.display()))?;
    for block in action_run_blocks(&document) {
        inspect_offline_commands(&block, action)?;
    }
    for nested in local_action_references(&document) {
        inspect_local_action(&nested, seen)?;
    }
    Ok(())
}

#[derive(Debug)]
struct RustFunction {
    name: String,
    attributes: String,
    body: String,
}

fn rust_functions(source: &str) -> Vec<RustFunction> {
    let mut functions = Vec::new();
    let mut attributes = Vec::new();
    let lines = source.lines().collect::<Vec<_>>();
    let mut index = 0usize;
    while index < lines.len() {
        let trimmed = lines[index].trim();
        if trimmed.starts_with("#[") {
            attributes.push(trimmed.to_string());
            index += 1;
            continue;
        }
        let Some(fn_index) = trimmed.find("fn ") else {
            if !trimmed.is_empty() && !trimmed.starts_with("pub ") && !trimmed.starts_with("async ")
            {
                attributes.clear();
            }
            index += 1;
            continue;
        };
        let name = trimmed[fn_index + 3..]
            .chars()
            .take_while(|character| character.is_ascii_alphanumeric() || *character == '_')
            .collect::<String>();
        if name.is_empty() {
            attributes.clear();
            index += 1;
            continue;
        }
        let start = index;
        let mut depth = 0isize;
        let mut opened = false;
        while index < lines.len() {
            for character in lines[index].chars() {
                if character == '{' {
                    depth += 1;
                    opened = true;
                } else if character == '}' && opened {
                    depth -= 1;
                }
            }
            index += 1;
            if opened && depth <= 0 {
                break;
            }
        }
        functions.push(RustFunction {
            name,
            attributes: attributes.join("\n"),
            body: lines[start..index].join("\n"),
        });
        attributes.clear();
    }
    functions
}

fn sensitive_test_functions(source: &str) -> Vec<(String, String)> {
    let functions = rust_functions(source);
    let mut sensitive = functions
        .iter()
        .filter(|function| {
            [
                "TcpListener",
                "TcpStream::connect",
                "UdpSocket",
                "UnixListener",
                "UnixStream::connect",
                "Command::new",
                "tokio::process",
                "std::process::Command",
                ".tcp_server(",
                "IpcServer::new",
            ]
            .iter()
            .any(|token| function.body.contains(token))
        })
        .map(|function| function.name.clone())
        .collect::<HashSet<_>>();
    loop {
        let before = sensitive.len();
        for function in &functions {
            if sensitive.iter().any(|name| function.body.contains(&format!("{name}("))) {
                sensitive.insert(function.name.clone());
            }
        }
        if sensitive.len() == before {
            break;
        }
    }
    functions
        .iter()
        .filter(|function| {
            (function.attributes.contains("#[test]")
                || function.attributes.contains("#[tokio::test"))
                && sensitive.contains(&function.name)
        })
        .map(|function| (function.name.clone(), function.attributes.clone()))
        .collect()
}

fn package_and_target(path: &str, packages: &HashMap<String, PathBuf>) -> Option<(String, String)> {
    let absolute = root().join(path);
    let (package, manifest) = packages
        .iter()
        .filter(|(_, manifest)| absolute.starts_with(manifest.parent().expect("manifest parent")))
        .max_by_key(|(_, manifest)| manifest.components().count())?;
    let package_root = manifest.parent().expect("manifest parent");
    let relative = absolute.strip_prefix(package_root).ok()?;
    let target = if relative.starts_with("tests") {
        relative.file_stem()?.to_string_lossy().into_owned()
    } else {
        "lib".to_string()
    };
    Some((package.clone(), target))
}

fn qualified_test_name(path: &str, target: &str, test_name: &str) -> String {
    if target != "lib" {
        return test_name.to_string();
    }
    let module = path
        .split("/src/")
        .nth(1)
        .unwrap_or(path)
        .trim_end_matches(".rs")
        .trim_end_matches("/mod")
        .replace('/', "::");
    format!("{module}::tests::{test_name}")
}

#[test]
fn ordinary_recipe_expansion_is_offline_and_has_no_shell_indirection() {
    let body_dependency_fixture =
        recipes("root:\n    just child\nchild:\n    python3 forbidden.py\n");
    let fixture_closure = recipe_closure("root", &body_dependency_fixture, &mut HashSet::new());
    assert!(
        fixture_closure.contains("python3 forbidden.py"),
        "body-invoked Just recipe was not expanded"
    );
    assert!(
        delegated_paths("bash scripts/upstream-review.sh").contains("scripts/upstream-review.sh"),
        "shell interpreter argument was not resolved"
    );

    let parsed = recipes(&read("justfile"));
    let closure = recipe_closure("validate", &parsed, &mut HashSet::new());
    let lower = closure.to_ascii_lowercase();
    for line in read("justfile").lines().filter(|line| !line.starts_with(char::is_whitespace)) {
        let trimmed = line.trim_start();
        assert!(
            !trimmed.starts_with("import ")
                && !trimmed.starts_with("mod ")
                && !trimmed.starts_with("alias ")
                && !trimmed.starts_with('['),
            "unsupported Just import/module/alias/attribute: {line}"
        );
    }
    for forbidden in [
        "pip ",
        "docker",
        "podman",
        "keychain",
        "yubikey",
        "serial",
        "generate-fixtures",
        "test-interop-full",
        "upstream-review",
        "--all-features",
        " --ignored",
        "cargo run",
        "bash ",
        " sh ",
        "$(",
        "`",
    ] {
        assert!(
            !lower.contains(forbidden),
            "ordinary validation contains '{forbidden}':\n{closure}"
        );
    }
    inspect_offline_commands(&closure, "validate").expect("ordinary commands obey offline policy");

    let policy = read("tests/offline-validation.toml");
    let excluded = quoted_values(&policy, "excluded_packages");
    let invocations = cargo_invocations(&closure);
    assert!(!invocations.is_empty(), "ordinary Cargo target model was vacuous");
    for invocation in &invocations {
        assert!(
            invocation.manifest_path.is_none(),
            "ordinary validation uses unsupported --manifest-path: {:?}",
            invocation.manifest_path
        );
    }
    for package in &excluded {
        assert!(
            closure.contains(&format!("--exclude {package}")),
            "ordinary validation does not exclude sensitive package {package}"
        );
        for invocation in invocations.iter().filter(|invocation| {
            invocation.workspace && matches!(invocation.command.as_str(), "test" | "clippy")
        }) {
            assert!(
                invocation.excludes.contains(package),
                "ordinary {} workspace selection includes sensitive package {package}",
                invocation.command
            );
        }
    }
    assert!(
        invocations
            .iter()
            .any(|invocation| invocation.command == "clippy" && invocation.all_targets),
        "ordinary Clippy target model did not inspect all selected targets"
    );
    let expected_matrix = quoted_values(&policy, "selected_targets").into_iter().collect::<HashSet<_>>();
    let actual_matrix = selected_test_matrix(&invocations);
    assert_eq!(actual_matrix, expected_matrix, "ordinary test target matrix drifted");
    for required in [
        "cargo test -p styrene-ipc-server --lib --test wire_compat",
        "--features interop-tests,transport",
        "scripts/test_validation_offline.rs",
    ] {
        assert!(closure.contains(required), "ordinary validation lost '{required}'");
    }

    let delegated = delegated_paths(&closure);
    assert!(!delegated.is_empty(), "ordinary validation delegation scan was vacuous");
    for path in delegated {
        let content = read(&path).to_ascii_lowercase();
        if path.ends_with(".rs") {
            for forbidden in
                ["use std::net", "use tokio::net", "use std::process", "use tokio::process"]
            {
                assert!(
                    !content.lines().any(|line| line.trim_start().starts_with(forbidden)),
                    "delegated {path} imports '{forbidden}'"
                );
            }
        } else {
            for forbidden in [
                "tcpstream::connect(",
                "tcplistener::bind(",
                "udpsocket::bind(",
                "command::new(",
                "curl ",
                "wget ",
                "git fetch",
            ] {
                assert!(!content.contains(forbidden), "delegated {path} contains '{forbidden}'");
            }
        }
    }
}

#[test]
fn sensitive_tests_have_specific_ignore_or_exclusion_evidence() {
    let policy = read("tests/offline-validation.toml");
    assert!(policy.contains("schema_version = 1"));
    let entries = sensitive_entries(&policy);
    assert!(entries.len() >= 10, "sensitive test inventory is unexpectedly empty");
    let parsed = recipes(&read("justfile"));
    let ordinary = recipe_closure("validate", &parsed, &mut HashSet::new());
    let ordinary_cargo = cargo_invocations(&ordinary);
    let safe_sensitive = quoted_values(&policy, "safe_sensitive_tests");
    let packages = workspace_packages();
    for package in quoted_values(&policy, "excluded_packages") {
        assert!(packages.contains_key(&package), "excluded package does not exist: {package}");
    }
    for selected in quoted_values(&policy, "selected_targets") {
        let (package, target) =
            selected.split_once(':').expect("selected target uses package:target");
        let manifest = packages
            .get(package)
            .unwrap_or_else(|| panic!("selected package does not exist: {package}"));
        let package_root = manifest.parent().expect("manifest has a parent");
        if target == "*" {
            // Package-wide selection intentionally includes every ordinary target.
        } else if target == "lib" {
            assert!(
                package_root.join("src/lib.rs").is_file(),
                "selected library does not exist: {selected}"
            );
        } else if let Some(test) = target.strip_prefix("test/") {
            assert!(
                package_root.join("tests").join(format!("{test}.rs")).is_file(),
                "selected test target does not exist: {selected}"
            );
        } else if let Some(bin) = target.strip_prefix("bin/") {
            assert!(
                package_root.join("src/main.rs").is_file()
                    || package_root.join("src/bin").join(format!("{bin}.rs")).is_file()
                    || package_root.join("src/bin").join(bin).join("main.rs").is_file(),
                "selected binary target does not exist: {selected}"
            );
        } else {
            panic!("selected target has unsupported selector: {selected}");
        }
    }
    let mut discovered = 0usize;

    for file in repository_files("crates", &["rs"]) {
        let path = relative(&file);
        let source = read(&path);
        let Some((package, target)) = package_and_target(&path, &packages) else { continue };
        for (test_name, attributes) in sensitive_test_functions(&source) {
            discovered += 1;
            let test_id = format!("{path}#{test_name}");
            let qualified = qualified_test_name(&path, &target, &test_name);
            let selected = ordinary_cargo
                .iter()
                .filter(|invocation| invocation.command == "test")
                .any(|invocation| invocation.selects_test(&package, &target, &qualified));
            if selected {
                assert!(
                    attributes.contains("#[ignore = \"") || safe_sensitive.contains(&test_id),
                    "ordinary validation selects sensitive test without reasoned ignore: {test_id}"
                );
            } else {
                let (_, evidence) = entries
                    .iter()
                    .find(|(inventory_path, _)| {
                        path == *inventory_path || path.starts_with(&format!("{inventory_path}/"))
                    })
                    .unwrap_or_else(|| {
                        panic!("sensitive test has no exclusion evidence: {test_id}")
                    });
                assert!(!evidence.trim().is_empty(), "empty evidence for {test_id}");
            }
        }
    }
    assert!(discovered > 0, "sensitive source discovery found no tests");

    for (path, evidence) in &entries {
        assert!(root().join(path).exists(), "inventory path does not exist: {path}");
        for part in evidence.split(';').map(str::trim) {
            if let Some(recipe) = part.strip_prefix("explicit-recipe:") {
                assert!(parsed.contains_key(recipe), "{path} references missing recipe {recipe}");
                assert!(!ordinary.contains(&format!("{recipe}:")), "{recipe} leaked into validate");
            }
            if let Some(workflow) = part.strip_prefix("manual-workflow:") {
                let content = read(format!(".github/workflows/{workflow}"));
                assert!(has_trigger(&content, "workflow_dispatch"));
                assert!(!has_trigger(&content, "push"));
                assert!(!has_trigger(&content, "pull_request"));
                assert!(!has_trigger(&content, "schedule"));
            }
        }
    }
}

#[test]
fn every_workflow_obeys_trigger_and_reuse_boundaries() {
    let workflows = repository_files(".github/workflows", &["yml", "yaml"]);
    assert!(!workflows.is_empty(), "workflow discovery must not be vacuous");
    let policy = read("tests/offline-validation.toml");
    let excluded = quoted_values(&policy, "excluded_packages");
    assert!(
        policy.contains("immutable_action_policy = \"reusable-workflows\""),
        "immutable action policy must be explicit"
    );
    let mut ordinary_count = 0usize;
    let mut release_count = 0usize;
    let mut external_action_count = 0usize;

    for file in workflows {
        let path = relative(&file);
        let workflow = read(&path);
        let lower = workflow.to_ascii_lowercase();
        let triggers = workflow_triggers(&workflow);
        let scheduled = triggers.contains("schedule");
        let release_operation = workflow.contains("# validation-class: release-operation");
        let repository_policy = workflow.contains("# validation-class: repository-policy");
        let validation_behavior = lower.contains("cargo test")
            || lower.contains("cargo check")
            || lower.contains("cargo clippy");
        let ordinary_validation = !release_operation
            && validation_behavior
            && (triggers.contains("push")
                || triggers.contains("pull_request")
                || triggers.contains("pull_request_target"));

        if release_operation {
            release_count += 1;
            assert!(
                lower.contains("cargo publish")
                    || lower.contains("release-plz")
                    || lower.contains("packages: write"),
                "release classification lacks release behavior: {path}"
            );
            assert!(
                triggers.contains("workflow_dispatch") || triggers.contains("push"),
                "release workflow has no operational trigger: {path}"
            );
            for forbidden in
                ["run: just validate", "uses: ./.github/workflows/ci", "ordinary-validation"]
            {
                assert!(
                    !lower.contains(forbidden),
                    "release workflow {path} invokes ordinary validation claim '{forbidden}'"
                );
            }
        }

        if scheduled {
            assert!(lower.contains("committed fixtures"), "scheduled {path} is not fixture-only");
            for forbidden in [
                "git fetch",
                "git push",
                "gh pr",
                "cargo publish",
                "release-plz",
                "setup-python",
                "pip install",
                "docker",
                "podman",
                "cargo install",
                "cargo audit",
            ] {
                assert!(!lower.contains(forbidden), "scheduled {path} contains '{forbidden}'");
            }
        }

        if ordinary_validation {
            ordinary_count += 1;
            for forbidden in [
                "--test mobile_node",
                "--test server_integration",
                "--ignored",
                "setup-python",
                "pip install",
                "docker",
                "podman",
            ] {
                assert!(
                    !lower.contains(forbidden),
                    "ordinary workflow {path} contains '{forbidden}'"
                );
            }
            for package in &excluded {
                assert!(
                    workflow.contains(&format!("--exclude {package}")),
                    "ordinary workflow {path} does not exclude {package}"
                );
            }
            for delegated in delegated_paths(&workflow) {
                if delegated == "scripts/test_validation_offline.rs" {
                    continue;
                }
                let content = read(&delegated).to_ascii_lowercase();
                for forbidden in ["curl ", "wget ", "git fetch", "docker ", "podman ", "python3 "] {
                    assert!(!content.contains(forbidden), "ordinary workflow {path} delegates '{forbidden}' through {delegated}");
                }
            }
            for invocation in cargo_invocations(&workflow) {
                if invocation.command == "test"
                    && invocation.packages.iter().any(|package| package == "styrene-interop-runner")
                {
                    assert_eq!(
                        invocation.tests,
                        ["rns_fixtures", "rns_handoff_manifests", "pinned_evidence_record"],
                        "ordinary workflow {path} selects live interoperability targets"
                    );
                }
                if invocation.command == "test"
                    && invocation.packages.iter().any(|package| package == "styrene-e2e")
                {
                    assert_eq!(
                        invocation.tests,
                        [
                            "identity",
                            "lxmf_protocol",
                            "pqc_scenario",
                        ],
                        "ordinary workflow {path} selects unapproved E2E targets"
                    );
                }
            }
            for action in local_action_references(&workflow) {
                inspect_local_action(&action, &mut HashSet::new()).unwrap_or_else(|error| {
                    panic!("ordinary workflow {path} violates local-action offline policy: {error}")
                });
            }
        }

        for line in workflow.lines() {
            if let Some(action) = line.trim().strip_prefix("- uses:") {
                let action = action.trim().trim_matches(['\'', '"']);
                if action.starts_with("./") {
                    let action_root = root().join(action.trim_start_matches("./"));
                    let manifest = [action_root.join("action.yml"), action_root.join("action.yaml")]
                        .into_iter()
                        .find(|candidate| candidate.is_file())
                        .unwrap_or_else(|| panic!("local action has no manifest in {path}: {action}"));
                    let local = fs::read_to_string(&manifest).expect("read local action manifest");
                    for delegated in delegated_paths(&local) {
                        assert!(root().join(delegated).is_file(), "local action delegates missing file");
                    }
                } else {
                    external_action_count += 1;
                    let (repository, reference) = action
                        .rsplit_once('@')
                        .unwrap_or_else(|| panic!("external action lacks a reference in {path}: {line}"));
                    assert!(
                        repository.split('/').count() >= 2 && !reference.is_empty(),
                        "invalid external action syntax in {path}: {line}"
                    );
                }
            }
            if let Some(reuse) = line.strip_prefix("    uses:") {
                let reference = reuse.trim().rsplit_once('@').map(|(_, value)| value).unwrap_or("");
                assert!(
                    reference.len() == 40 && reference.bytes().all(|byte| byte.is_ascii_hexdigit()),
                    "job-level reusable workflow is not pinned by commit in {path}: {line}"
                );
            }
        }

        let sensitive = [
            "cargo test -p styrene-e2e",
            "cargo test -p styrene-interop-runner",
            "--test mobile_node",
            "docker compose",
            "setup-python",
            "cargo publish",
            "git push",
            "gh pr create",
        ]
        .iter()
        .any(|token| lower.contains(token));
        if sensitive && !release_operation && !ordinary_validation && !repository_policy {
            assert!(
                has_trigger(&workflow, "workflow_dispatch"),
                "sensitive workflow {path} is not manual"
            );
            assert!(!has_trigger(&workflow, "schedule"), "sensitive workflow {path} is scheduled");
        }
    }
    assert!(ordinary_count > 0, "ordinary workflow classification was vacuous");
    assert!(release_count > 0, "release workflow classification was vacuous");
    assert!(external_action_count > 0, "external action policy check was vacuous");
}

#[test]
fn adversarial_parser_forms_fail_closed() {
    assert!(std::panic::catch_unwind(|| cargo_invocations("cargo test -pstyrened")).is_err());
    assert_eq!(
        workflow_triggers("name: fixture\non: [push, 'pull_request_target']\njobs: {}"),
        HashSet::from(["push".to_string(), "pull_request_target".to_string()])
    );
    let body = recipes("root:\n    just child\nchild:\n    cargo run -p live\n");
    assert!(recipe_closure("root", &body, &mut HashSet::new()).contains("cargo run -p live"));
    let expected = HashSet::from(["safe:test/unit".to_string()]);
    let actual = selected_test_matrix(&cargo_invocations(
        "cargo test -p safe --test unit --test unreviewed",
    ));
    assert_ne!(actual, expected, "an extra selected target must change the exact matrix");
    let script_fixture = "bash tests/fixtures/offline-validation/python-wrapper.sh";
    assert!(inspect_offline_commands(script_fixture, "script fixture").is_err());
    let action_error = inspect_local_action(
        "./tests/fixtures/offline-validation/outer-action",
        &mut HashSet::new(),
    )
    .expect_err("nested local action Python bypass must fail closed");
    assert!(action_error.contains("interpreter is forbidden"), "{action_error}");
}

#[test]
fn parity_gate_inventory_is_non_vacuous_and_live_gates_are_isolated() {
    let product = read("product/capabilities-v1.toml");
    let parsed = recipes(&read("justfile"));
    let ordinary = recipe_closure("validate", &parsed, &mut HashSet::new());
    let gates = table_blocks(&product, "[[parity_gates]]");
    assert!(gates.len() >= 8, "parity gate table is unexpectedly empty");
    let mut ids = HashSet::new();
    let mut fixture_count = 0usize;
    let mut live_count = 0usize;
    for block in gates {
        let id = block
            .lines()
            .find_map(|line| line.strip_prefix("id = \"").and_then(|value| value.strip_suffix('"')))
            .expect("parity gate has an id");
        assert!(ids.insert(id), "duplicate parity gate {id}");
        if block.contains("kind = \"fixture\"") {
            fixture_count += 1;
            assert!(block.contains("enabled = true"));
            assert!(block.contains("ignored = false"));
            assert!(!block.to_ascii_lowercase().contains("python3"));
        }
        if block.contains("kind = \"live\"") || block.contains("kind = \"manual\"") {
            live_count += 1;
            assert!(block.contains("enabled = false"), "live gate enabled: {id}");
            assert!(block.contains("ignored = true"), "live gate not ignored: {id}");
            if block.contains("automated = true") {
                assert!(block.contains("--ignored"), "automated live gate lacks opt-in: {id}");
            }
        }
    }
    assert!(fixture_count > 0, "fixture gate checks were vacuous");
    assert!(live_count > 0, "live gate checks were vacuous");
    for (gate, command_fragment) in [
        ("rns-python-fixtures", "--features interop-tests,transport"),
        ("rns-parity-contracts", "--test rns_fixtures --test rns_handoff_manifests"),
        ("lxmf-rust-codec", "--test lxmf_protocol"),
        ("micron-conformance", "-p styrene-micron"),
        ("nomadnet-styrene-pages", "--test nomadnet_pages_offline"),
    ] {
        assert!(ids.contains(gate), "required offline parity gate is missing: {gate}");
        assert!(
            ordinary.contains(command_fragment),
            "enabled offline parity gate {gate} is unreachable from validate"
        );
    }
}

#[test]
fn rns_parity_policy_rejects_unsafe_validation_and_claim_promotion() {
    let index_path = "tests/interop/fixtures/rns/index-v2.json";
    let index = read(index_path);
    let consumers = read("tests/interop/fixtures/rns/consumers-v1.json");
    let handoff = read("tests/interop/handoffs/reticulum-1.5.1-live.json");
    let product = read("product/capabilities-v1.toml");
    let rns_json = repository_files("tests/interop/fixtures/rns", &["json"])
        .into_iter()
        .map(|path| {
            let relative = relative(&path);
            let document = fs::read_to_string(path).expect("read RNS JSON document");
            (relative, document)
        })
        .collect::<Vec<_>>();
    let authority_documents = rns_json
        .iter()
        .map(|(path, document)| (path.as_str(), document.as_str()))
        .collect::<Vec<_>>();
    assert!(
        rns_parity_policy_errors(&index, &consumers, &handoff, &product, &authority_documents)
            .is_empty()
    );
    for (path, authority_ids) in [
        (
            "openspec/archive/2026-09-02-beechat-rns-corrections-wave/specs/rns-wire-corrections.md",
            ["rns-1.5.1", "rns-1.5.1"],
        ),
        (
            "openspec/archive/2026-09-02-freetak-rns-hardening-wave/specs/rns-security-hardening.md",
            ["rns-1.5.1", "rns-1.5.1"],
        ),
        (
            "openspec/changes/leviculum-rns-corpus-wave/specs/rns-corpus-governance.md",
            ["rns-1.4.2", "rns-1.5.1"],
        ),
    ] {
        let contract = read(path);
        assert!(contract.contains("styrene_interop_runner::rns_fixtures"));
        assert!(contract.contains("tests/interop/fixtures/rns/index-v2.json"));
        assert!(authority_ids.iter().all(|id| contract.contains(id)));
    }

    assert!(inspect_offline_commands("curl https://example.invalid", "network mutation").is_err());
    assert!(inspect_offline_commands("python3 generate.py", "Python launch").is_err());
    assert!(enables_hardware_by_default("default = [\"serial\"]"));

    let mutable = handoff.replace(
        "149e4151095adf098b8f53eab0c03b37169e8559",
        "main",
    );
    assert!(!rns_parity_policy_errors(
        &index,
        &consumers,
        &mutable,
        &product,
        &authority_documents,
    )
    .is_empty());

    let mut competing = authority_documents.clone();
    competing.push(("tests/interop/fixtures/rns/other.json", "{\"authorities\":{}}"));
    assert!(!rns_parity_policy_errors(&index, &consumers, &handoff, &product, &competing).is_empty());

    let no_checksum = handoff.replacen("\"artifact_sha256_required\": true", "", 1);
    assert!(!rns_parity_policy_errors(
        &index,
        &consumers,
        &no_checksum,
        &product,
        &authority_documents,
    )
    .is_empty());

    let promoted = handoff.replace(
        "\"claim_status\": \"unevidenced\"",
        "\"claim_status\": \"supported\"",
    );
    assert!(!rns_parity_policy_errors(
        &index,
        &consumers,
        &promoted,
        &product,
        &authority_documents,
    )
    .is_empty());

    let registered = product.clone()
        + "\n[[parity_gates]]\nid = \"rns-1.5.1-mixed-interface-mtu\"\n";
    assert!(!rns_parity_policy_errors(
        &index,
        &consumers,
        &handoff,
        &registered,
        &authority_documents,
    )
    .is_empty());
}

#[test]
fn hardware_features_remain_opt_in() {
    for (path, declaration) in [
        ("crates/libs/styrene-entropy/Cargo.toml", "hardware-trng = [\"dep:serialport\"]"),
        ("crates/libs/styrene-rns/Cargo.toml", "serial = [\"transport\", \"dep:tokio-serial\"]"),
        ("crates/libs/styrene-identity/Cargo.toml", "yubikey = ["),
        ("crates/libs/styrene-identity/Cargo.toml", "keychain = ["),
    ] {
        let manifest = read(path);
        assert!(manifest.contains(declaration), "{path} lost '{declaration}'");
        assert!(!enables_hardware_by_default(&manifest), "{path} enables hardware by default");
    }
}
