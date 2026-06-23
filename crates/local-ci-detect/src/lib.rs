use local_ci_core::{
    CacheConfig, Config, DepsConfig, RemoteSSHDefaults, Stage, Workspace, WorkspaceConfig,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProjectType {
    Rust,
    Python,
    TypeScript,
    Go,
    Java,
    Swift,
    Generic,
}

impl std::fmt::Display for ProjectType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            ProjectType::Rust => "rust",
            ProjectType::Python => "python",
            ProjectType::TypeScript => "typescript",
            ProjectType::Go => "go",
            ProjectType::Java => "java",
            ProjectType::Swift => "swift",
            ProjectType::Generic => "generic",
        };
        write!(f, "{}", s)
    }
}

pub fn detect_project_type(root: &Path) -> ProjectType {
    if root.join("Cargo.toml").exists() {
        return ProjectType::Rust;
    }
    if root.join("package.json").exists() {
        return ProjectType::TypeScript;
    }
    if root.join("Package.swift").exists() {
        return ProjectType::Swift;
    }

    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.flatten() {
            if let Ok(file_type) = entry.file_type() {
                if file_type.is_dir() {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    if name.ends_with(".xcodeproj") || name.ends_with(".xcworkspace") {
                        return ProjectType::Swift;
                    }
                }
            }
        }
    }

    if root.join("pyproject.toml").exists()
        || root.join("setup.py").exists()
        || root.join("requirements.txt").exists()
    {
        return ProjectType::Python;
    }

    if root.join("go.mod").exists() {
        return ProjectType::Go;
    }

    if root.join("pom.xml").exists() || root.join("build.gradle").exists() {
        return ProjectType::Java;
    }

    ProjectType::Generic
}

pub fn is_command_in_path(cmd: &str) -> bool {
    if let Some(paths) = std::env::var_os("PATH") {
        for path in std::env::split_paths(&paths) {
            let p = path.join(cmd);
            if p.is_file() {
                return true;
            }
        }
    }
    false
}

pub fn has_cargo_nextest() -> bool {
    // Check if cargo-nextest is in PATH
    is_command_in_path("cargo-nextest")
}

pub fn default_test_command() -> Vec<String> {
    if has_cargo_nextest() {
        vec![
            "cargo".to_string(),
            "nextest".to_string(),
            "run".to_string(),
            "--workspace".to_string(),
        ]
    } else {
        vec![
            "cargo".to_string(),
            "test".to_string(),
            "--workspace".to_string(),
        ]
    }
}

pub fn get_default_stages_for_type(
    project_type: ProjectType,
    root: &Path,
) -> HashMap<String, Stage> {
    let mut stages = HashMap::new();

    match project_type {
        ProjectType::Rust => {
            stages.insert(
                "fmt".to_string(),
                Stage {
                    name: "fmt".to_string(),
                    command: Some(vec![
                        "cargo".to_string(),
                        "fmt".to_string(),
                        "--all".to_string(),
                        "--".to_string(),
                        "--check".to_string(),
                    ]),
                    fix_command: Some(vec![
                        "cargo".to_string(),
                        "fmt".to_string(),
                        "--all".to_string(),
                    ]),
                    check: true,
                    timeout: 120,
                    enabled: true,
                    depends_on: vec![],
                    watch: vec!["*.rs".to_string()],
                },
            );
            stages.insert(
                "clippy".to_string(),
                Stage {
                    name: "clippy".to_string(),
                    command: Some(vec![
                        "cargo".to_string(),
                        "clippy".to_string(),
                        "--workspace".to_string(),
                        "--all-targets".to_string(),
                        "--".to_string(),
                        "-D".to_string(),
                        "warnings".to_string(),
                    ]),
                    fix_command: None,
                    check: false,
                    timeout: 600,
                    enabled: true,
                    depends_on: vec!["fmt".to_string()],
                    watch: vec![
                        "*.rs".to_string(),
                        "Cargo.toml".to_string(),
                        "Cargo.lock".to_string(),
                    ],
                },
            );
            stages.insert(
                "test".to_string(),
                Stage {
                    name: "test".to_string(),
                    command: Some(default_test_command()),
                    fix_command: None,
                    check: false,
                    timeout: 1200,
                    enabled: true,
                    depends_on: vec!["fmt".to_string()],
                    watch: vec![
                        "*.rs".to_string(),
                        "Cargo.toml".to_string(),
                        "Cargo.lock".to_string(),
                    ],
                },
            );
            stages.insert(
                "check".to_string(),
                Stage {
                    name: "check".to_string(),
                    command: Some(vec![
                        "cargo".to_string(),
                        "check".to_string(),
                        "--workspace".to_string(),
                    ]),
                    fix_command: None,
                    check: false,
                    timeout: 600,
                    enabled: false,
                    depends_on: vec![],
                    watch: vec![
                        "*.rs".to_string(),
                        "Cargo.toml".to_string(),
                        "Cargo.lock".to_string(),
                    ],
                },
            );
            stages.insert(
                "deny".to_string(),
                Stage {
                    name: "deny".to_string(),
                    command: Some(vec![
                        "cargo".to_string(),
                        "deny".to_string(),
                        "check".to_string(),
                    ]),
                    fix_command: None,
                    check: false,
                    timeout: 300,
                    enabled: false,
                    depends_on: vec![],
                    watch: vec![
                        "Cargo.toml".to_string(),
                        "Cargo.lock".to_string(),
                        "deny.toml".to_string(),
                    ],
                },
            );
            stages.insert(
                "audit".to_string(),
                Stage {
                    name: "audit".to_string(),
                    command: Some(vec!["cargo".to_string(), "audit".to_string()]),
                    fix_command: None,
                    check: false,
                    timeout: 300,
                    enabled: false,
                    depends_on: vec![],
                    watch: vec!["Cargo.toml".to_string(), "Cargo.lock".to_string()],
                },
            );
            stages.insert(
                "machete".to_string(),
                Stage {
                    name: "machete".to_string(),
                    command: Some(vec!["cargo".to_string(), "machete".to_string()]),
                    fix_command: None,
                    check: false,
                    timeout: 300,
                    enabled: false,
                    depends_on: vec![],
                    watch: vec!["*.rs".to_string(), "Cargo.toml".to_string()],
                },
            );
            stages.insert(
                "taplo".to_string(),
                Stage {
                    name: "taplo".to_string(),
                    command: Some(vec![
                        "taplo".to_string(),
                        "format".to_string(),
                        "--check".to_string(),
                        ".".to_string(),
                    ]),
                    fix_command: Some(vec![
                        "taplo".to_string(),
                        "format".to_string(),
                        ".".to_string(),
                    ]),
                    check: true,
                    timeout: 300,
                    enabled: false,
                    depends_on: vec![],
                    watch: vec!["*.toml".to_string()],
                },
            );
        }
        ProjectType::TypeScript => {
            stages.insert(
                "install".to_string(),
                Stage {
                    name: "install".to_string(),
                    command: Some(vec!["bun".to_string(), "install".to_string()]),
                    fix_command: None,
                    check: false,
                    timeout: 300,
                    enabled: true,
                    depends_on: vec![],
                    watch: vec![
                        "package.json".to_string(),
                        "bun.lock".to_string(),
                        "bun.lockb".to_string(),
                    ],
                },
            );
            stages.insert(
                "typecheck".to_string(),
                Stage {
                    name: "typecheck".to_string(),
                    command: Some(vec![
                        "bun".to_string(),
                        "x".to_string(),
                        "tsc".to_string(),
                        "--noEmit".to_string(),
                    ]),
                    fix_command: None,
                    check: false,
                    timeout: 120,
                    enabled: true,
                    depends_on: vec!["install".to_string()],
                    watch: vec![
                        "*.ts".to_string(),
                        "*.tsx".to_string(),
                        "*.json".to_string(),
                    ],
                },
            );
            stages.insert(
                "lint".to_string(),
                Stage {
                    name: "lint".to_string(),
                    command: Some(vec![
                        "bun".to_string(),
                        "run".to_string(),
                        "lint".to_string(),
                    ]),
                    fix_command: Some(vec![
                        "bun".to_string(),
                        "run".to_string(),
                        "lint".to_string(),
                        "--".to_string(),
                        "--fix".to_string(),
                    ]),
                    check: false,
                    timeout: 300,
                    enabled: true,
                    depends_on: vec!["install".to_string()],
                    watch: vec!["*.js".to_string(), "*.ts".to_string(), "*.json".to_string()],
                },
            );
            stages.insert(
                "test".to_string(),
                Stage {
                    name: "test".to_string(),
                    command: Some(vec!["bun".to_string(), "test".to_string()]),
                    fix_command: None,
                    check: false,
                    timeout: 600,
                    enabled: true,
                    depends_on: vec!["install".to_string()],
                    watch: vec!["*.js".to_string(), "*.ts".to_string(), "*.json".to_string()],
                },
            );
            stages.insert(
                "format".to_string(),
                Stage {
                    name: "format".to_string(),
                    command: Some(vec![
                        "bun".to_string(),
                        "run".to_string(),
                        "format".to_string(),
                        "--check".to_string(),
                    ]),
                    fix_command: Some(vec![
                        "bun".to_string(),
                        "run".to_string(),
                        "format".to_string(),
                    ]),
                    check: true,
                    timeout: 120,
                    enabled: false,
                    depends_on: vec!["install".to_string()],
                    watch: vec!["*.js".to_string(), "*.ts".to_string(), "*.json".to_string()],
                },
            );
        }
        ProjectType::Swift => {
            let is_spm = root.join("Package.swift").exists();
            stages.insert(
                "fmt".to_string(),
                Stage {
                    name: "fmt".to_string(),
                    command: Some(vec![
                        "swift-format".to_string(),
                        "lint".to_string(),
                        "--recursive".to_string(),
                        ".".to_string(),
                    ]),
                    fix_command: Some(vec![
                        "swift-format".to_string(),
                        "format".to_string(),
                        "--in-place".to_string(),
                        "--recursive".to_string(),
                        ".".to_string(),
                    ]),
                    check: true,
                    timeout: 120,
                    enabled: true,
                    depends_on: vec![],
                    watch: vec!["*.swift".to_string()],
                },
            );
            stages.insert(
                "lint".to_string(),
                Stage {
                    name: "lint".to_string(),
                    command: Some(vec!["swiftlint".to_string(), "--strict".to_string()]),
                    fix_command: None,
                    check: false,
                    timeout: 300,
                    enabled: false,
                    depends_on: vec![],
                    watch: vec!["*.swift".to_string()],
                },
            );

            if is_spm {
                stages.insert(
                    "build".to_string(),
                    Stage {
                        name: "build".to_string(),
                        command: Some(vec!["swift".to_string(), "build".to_string()]),
                        fix_command: None,
                        check: false,
                        timeout: 600,
                        enabled: true,
                        depends_on: vec![],
                        watch: vec![
                            "*.swift".to_string(),
                            "Package.swift".to_string(),
                            "Package.resolved".to_string(),
                        ],
                    },
                );
                stages.insert(
                    "test".to_string(),
                    Stage {
                        name: "test".to_string(),
                        command: Some(vec!["swift".to_string(), "test".to_string()]),
                        fix_command: None,
                        check: false,
                        timeout: 1200,
                        enabled: true,
                        depends_on: vec![],
                        watch: vec![
                            "*.swift".to_string(),
                            "Package.swift".to_string(),
                            "Package.resolved".to_string(),
                        ],
                    },
                );
            } else {
                let scheme = "Placeholder".to_string();
                stages.insert(
                    "build".to_string(),
                    Stage {
                        name: "build".to_string(),
                        command: Some(vec![
                            "xcodebuild".to_string(),
                            "-scheme".to_string(),
                            scheme.clone(),
                            "build".to_string(),
                        ]),
                        fix_command: None,
                        check: false,
                        timeout: 600,
                        enabled: true,
                        depends_on: vec![],
                        watch: vec![
                            "*.swift".to_string(),
                            "*.xcconfig".to_string(),
                            "project.pbxproj".to_string(),
                        ],
                    },
                );
                stages.insert(
                    "test".to_string(),
                    Stage {
                        name: "test".to_string(),
                        command: Some(vec![
                            "xcodebuild".to_string(),
                            "test".to_string(),
                            "-scheme".to_string(),
                            scheme,
                            "-destination".to_string(),
                            "platform=macOS".to_string(),
                        ]),
                        fix_command: None,
                        check: false,
                        timeout: 1200,
                        enabled: true,
                        depends_on: vec![],
                        watch: vec![
                            "*.swift".to_string(),
                            "*.xcconfig".to_string(),
                            "project.pbxproj".to_string(),
                        ],
                    },
                );
            }
        }
        ProjectType::Python => {
            stages.insert(
                "lint".to_string(),
                Stage {
                    name: "lint".to_string(),
                    command: Some(vec![
                        "pylint".to_string(),
                        ".".to_string(),
                        "--errors-only".to_string(),
                    ]),
                    fix_command: None,
                    check: false,
                    timeout: 300,
                    enabled: false,
                    depends_on: vec![],
                    watch: vec!["*.py".to_string()],
                },
            );
            stages.insert(
                "format".to_string(),
                Stage {
                    name: "format".to_string(),
                    command: Some(vec![
                        "black".to_string(),
                        "--check".to_string(),
                        ".".to_string(),
                    ]),
                    fix_command: Some(vec!["black".to_string(), ".".to_string()]),
                    check: true,
                    timeout: 120,
                    enabled: false,
                    depends_on: vec![],
                    watch: vec!["*.py".to_string()],
                },
            );
            stages.insert(
                "test".to_string(),
                Stage {
                    name: "test".to_string(),
                    command: Some(vec!["pytest".to_string()]),
                    fix_command: None,
                    check: false,
                    timeout: 600,
                    enabled: false,
                    depends_on: vec![],
                    watch: vec!["*.py".to_string(), "pyproject.toml".to_string()],
                },
            );
        }
        ProjectType::Go => {
            stages.insert(
                "fmt".to_string(),
                Stage {
                    name: "fmt".to_string(),
                    command: Some(vec![
                        "go".to_string(),
                        "fmt".to_string(),
                        "./...".to_string(),
                    ]),
                    fix_command: Some(vec![
                        "go".to_string(),
                        "fmt".to_string(),
                        "./...".to_string(),
                    ]),
                    check: true,
                    timeout: 120,
                    enabled: false,
                    depends_on: vec![],
                    watch: vec!["*.go".to_string()],
                },
            );
            stages.insert(
                "vet".to_string(),
                Stage {
                    name: "vet".to_string(),
                    command: Some(vec![
                        "go".to_string(),
                        "vet".to_string(),
                        "./...".to_string(),
                    ]),
                    fix_command: None,
                    check: false,
                    timeout: 300,
                    enabled: false,
                    depends_on: vec![],
                    watch: vec![
                        "*.go".to_string(),
                        "go.mod".to_string(),
                        "go.sum".to_string(),
                    ],
                },
            );
            stages.insert(
                "test".to_string(),
                Stage {
                    name: "test".to_string(),
                    command: Some(vec![
                        "go".to_string(),
                        "test".to_string(),
                        "./...".to_string(),
                    ]),
                    fix_command: None,
                    check: false,
                    timeout: 600,
                    enabled: false,
                    depends_on: vec![],
                    watch: vec![
                        "*.go".to_string(),
                        "go.mod".to_string(),
                        "go.sum".to_string(),
                    ],
                },
            );
        }
        ProjectType::Java => {
            stages.insert(
                "build".to_string(),
                Stage {
                    name: "build".to_string(),
                    command: Some(vec![
                        "mvn".to_string(),
                        "clean".to_string(),
                        "compile".to_string(),
                    ]),
                    fix_command: None,
                    check: false,
                    timeout: 600,
                    enabled: false,
                    depends_on: vec![],
                    watch: vec!["*.java".to_string(), "pom.xml".to_string()],
                },
            );
            stages.insert(
                "test".to_string(),
                Stage {
                    name: "test".to_string(),
                    command: Some(vec!["mvn".to_string(), "test".to_string()]),
                    fix_command: None,
                    check: false,
                    timeout: 900,
                    enabled: false,
                    depends_on: vec![],
                    watch: vec!["*.java".to_string(), "pom.xml".to_string()],
                },
            );
        }
        ProjectType::Generic => {
            stages.insert(
                "test".to_string(),
                Stage {
                    name: "test".to_string(),
                    command: Some(vec![
                        "echo".to_string(),
                        "Please configure stages in .local-ci.toml".to_string(),
                    ]),
                    fix_command: None,
                    check: false,
                    timeout: 60,
                    enabled: false,
                    depends_on: vec![],
                    watch: vec![],
                },
            );
        }
    }

    stages
}

pub fn get_cache_pattern_for_type(project_type: ProjectType) -> Vec<String> {
    match project_type {
        ProjectType::Rust => vec![
            "*.rs".to_string(),
            "*.toml".to_string(),
            "*.lock".to_string(),
        ],
        ProjectType::Python => vec![
            "*.py".to_string(),
            "*.toml".to_string(),
            "*.txt".to_string(),
            "*.yml".to_string(),
            "*.yaml".to_string(),
        ],
        ProjectType::TypeScript => vec![
            "*.ts".to_string(),
            "*.tsx".to_string(),
            "*.js".to_string(),
            "*.jsx".to_string(),
            "*.json".to_string(),
            "package.json".to_string(),
            "tsconfig.json".to_string(),
            "bunfig.toml".to_string(),
            "bun.lock".to_string(),
            "bun.lockb".to_string(),
        ],
        ProjectType::Go => vec![
            "*.go".to_string(),
            "go.mod".to_string(),
            "go.sum".to_string(),
        ],
        ProjectType::Java => vec![
            "*.java".to_string(),
            "pom.xml".to_string(),
            "build.gradle".to_string(),
        ],
        ProjectType::Swift => vec![
            "*.swift".to_string(),
            "Package.swift".to_string(),
            "Package.resolved".to_string(),
            "*.xcconfig".to_string(),
            "project.pbxproj".to_string(),
        ],
        ProjectType::Generic => vec!["*".to_string()],
    }
}

pub fn get_skip_dirs_for_type(project_type: ProjectType) -> Vec<String> {
    let mut base_skip = vec![
        ".git".to_string(),
        ".github".to_string(),
        "scripts".to_string(),
        ".claude".to_string(),
        ".venv".to_string(),
        "venv".to_string(),
    ];

    match project_type {
        ProjectType::Rust => {
            base_skip.push("target".to_string());
        }
        ProjectType::Python => {
            base_skip.push(".pytest_cache".to_string());
            base_skip.push("__pycache__".to_string());
            base_skip.push(".mypy_cache".to_string());
        }
        ProjectType::TypeScript => {
            base_skip.push("node_modules".to_string());
            base_skip.push("dist".to_string());
            base_skip.push(".next".to_string());
            base_skip.push("coverage".to_string());
        }
        ProjectType::Go => {
            base_skip.push("vendor".to_string());
        }
        ProjectType::Java => {
            base_skip.push("target".to_string());
            base_skip.push("build".to_string());
        }
        ProjectType::Swift => {
            base_skip.push(".build".to_string());
            base_skip.push(".swiftpm".to_string());
            base_skip.push("DerivedData".to_string());
            base_skip.push("Pods".to_string());
        }
        ProjectType::Generic => {
            base_skip.push("node_modules".to_string());
            base_skip.push("target".to_string());
            base_skip.push("build".to_string());
            base_skip.push("dist".to_string());
        }
    }

    base_skip
}

pub fn detect_workspace(root: &Path) -> Result<Workspace, String> {
    let cargo_path = root.join("Cargo.toml");
    if cargo_path.exists() {
        return detect_cargo_workspace(root);
    }

    let package_path = root.join("package.json");
    if package_path.exists() && detect_project_type(root) == ProjectType::TypeScript {
        return detect_typescript_workspace(root);
    }

    let kind = detect_project_type(root);
    if kind == ProjectType::Swift {
        return detect_swift_workspace(root);
    }

    if root.join("go.mod").exists() {
        return Ok(Workspace {
            root: root.to_path_buf(),
            members: vec![".".to_string()],
            excludes: vec![],
            is_single: true,
        });
    }

    Ok(Workspace {
        root: root.to_path_buf(),
        members: vec![".".to_string()],
        excludes: vec![],
        is_single: true,
    })
}

#[derive(Deserialize)]
struct CargoTomlWorkspace {
    members: Option<Vec<String>>,
    exclude: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct CargoToml {
    workspace: Option<CargoTomlWorkspace>,
    package: Option<serde_json::Value>,
}

fn detect_cargo_workspace(root: &Path) -> Result<Workspace, String> {
    let cargo_path = root.join("Cargo.toml");
    let content =
        fs::read_to_string(&cargo_path).map_err(|e| format!("Failed to read Cargo.toml: {}", e))?;

    let cargo: CargoToml =
        toml::from_str(&content).map_err(|e| format!("Failed to parse Cargo.toml: {}", e))?;

    let mut ws = Workspace {
        root: root.to_path_buf(),
        members: vec![],
        excludes: vec![],
        is_single: false,
    };

    if let Some(workspace) = cargo.workspace {
        if let Some(members) = workspace.members {
            ws.members = expand_glob_patterns(root, &members)?;
        }
        if let Some(exclude) = workspace.exclude {
            ws.excludes = expand_glob_patterns(root, &exclude)?;
        }
    } else if cargo.package.is_some() {
        ws.is_single = true;
        ws.members = vec![".".to_string()];
    } else {
        return Err("Cargo.toml is neither a workspace nor a package".to_string());
    }

    Ok(ws)
}

#[derive(Deserialize)]
struct PackageJSON {
    name: Option<String>,
    workspaces: Option<Vec<String>>,
}

fn detect_typescript_workspace(root: &Path) -> Result<Workspace, String> {
    let package_path = root.join("package.json");
    let content = fs::read_to_string(&package_path)
        .map_err(|e| format!("Failed to read package.json: {}", e))?;

    let pkg: PackageJSON = serde_json::from_str(&content)
        .map_err(|e| format!("Failed to parse package.json: {}", e))?;

    let mut ws = Workspace {
        root: root.to_path_buf(),
        members: vec![],
        excludes: vec![],
        is_single: false,
    };

    let workspaces = match pkg.workspaces {
        Some(w) => w,
        None => {
            ws.is_single = true;
            let name = pkg.name.unwrap_or_else(|| {
                root.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned()
            });
            ws.members = vec![name];
            return Ok(ws);
        }
    };

    for pattern in workspaces {
        let full_pattern = root.join(&pattern);
        let pattern_str = full_pattern.to_string_lossy();
        if let Ok(paths) = glob::glob(&pattern_str) {
            for entry in paths.flatten() {
                if entry.is_dir() && entry.join("package.json").exists() {
                    if let Ok(rel) = entry.strip_prefix(root) {
                        ws.members.push(rel.to_string_lossy().into_owned());
                    }
                }
            }
        }
    }

    ws.members.sort();
    Ok(ws)
}

fn detect_swift_workspace(root: &Path) -> Result<Workspace, String> {
    let mut ws = Workspace {
        root: root.to_path_buf(),
        members: vec![],
        excludes: vec![],
        is_single: false,
    };

    if root.join("Package.swift").exists() {
        // Try swift package describe --type json
        let cmd = std::process::Command::new("swift")
            .args(["package", "describe", "--type", "json"])
            .current_dir(root)
            .output();

        if let Ok(output) = cmd {
            if output.status.success() {
                #[derive(Deserialize)]
                struct SwiftTarget {
                    name: String,
                }
                #[derive(Deserialize)]
                struct SwiftPackage {
                    targets: Vec<SwiftTarget>,
                }

                if let Ok(pkg) = serde_json::from_slice::<SwiftPackage>(&output.stdout) {
                    for target in pkg.targets {
                        ws.members.push(target.name);
                    }
                    ws.members.sort();
                    return Ok(ws);
                }
            }
        }

        ws.is_single = true;
        ws.members = vec![".".to_string()];
        return Ok(ws);
    }

    // Xcode fallback
    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.ends_with(".xcodeproj") || name.ends_with(".xcworkspace") {
                ws.members.push(name);
            }
        }
    }

    if ws.members.is_empty() {
        ws.is_single = true;
        ws.members = vec![".".to_string()];
    } else {
        ws.members.sort();
    }

    Ok(ws)
}

fn expand_glob_patterns(root: &Path, patterns: &[String]) -> Result<Vec<String>, String> {
    let mut result = Vec::new();

    for pattern in patterns {
        if !pattern.contains('*') && !pattern.contains('?') && !pattern.contains('[') {
            result.push(pattern.clone());
            continue;
        }

        let full_pattern = root.join(pattern);
        let pattern_str = full_pattern.to_string_lossy();

        let paths = glob::glob(&pattern_str)
            .map_err(|e| format!("Invalid glob pattern {}: {}", pattern, e))?;

        for entry in paths.flatten() {
            if let Ok(rel) = entry.strip_prefix(root) {
                result.push(rel.to_string_lossy().into_owned());
            }
        }
    }

    Ok(result)
}

pub fn get_config_template_for_type(project_type: ProjectType, root: &Path) -> String {
    match project_type {
        ProjectType::Rust => r#"# local-ci configuration for Rust project
# See: https://github.com/stevedores-org/local-ci

[cache]
skip_dirs = [".git", "target", ".github", "scripts", ".claude", "node_modules"]
include_patterns = ["*.rs", "*.toml", "*.lock"]

[stages.fmt]
command = ["cargo", "fmt", "--all", "--", "--check"]
fix_command = ["cargo", "fmt", "--all"]
timeout = 120
enabled = true

[stages.clippy]
command = ["cargo", "clippy", "--workspace", "--all-targets", "--", "-D", "warnings"]
timeout = 600
enabled = true

[stages.test]
command = ["cargo", "test", "--workspace"]
timeout = 1200
enabled = true

[stages.check]
command = ["cargo", "check", "--workspace"]
timeout = 600
enabled = false

[dependencies]
optional = []

[workspace]
exclude = []
"#.to_string(),
        ProjectType::TypeScript => r#"# local-ci configuration for TypeScript/Bun projects
# See: https://github.com/stevedores-org/local-ci
# Runtime: bun

[cache]
skip_dirs = [".git", "node_modules", "dist", ".next", "coverage", ".claude"]
include_patterns = ["*.ts", "*.tsx", "*.js", "*.jsx", "*.json", "package.json", "tsconfig.json", "bunfig.toml", "bun.lock", "bun.lockb"]

[stages.install]
command = ["bun", "install"]
timeout = 300
enabled = true

[stages.typecheck]
command = ["bun", "x", "tsc", "--noEmit"]
timeout = 120
enabled = true
depends_on = ["install"]

[stages.lint]
command = ["bun", "run", "lint"]
fix_command = ["bun", "run", "lint", "--", "--fix"]
timeout = 300
enabled = true
depends_on = ["install"]

[stages.test]
command = ["bun", "test"]
timeout = 600
enabled = true
depends_on = ["install"]

[stages.format]
command = ["bun", "run", "format", "--check"]
fix_command = ["bun", "run", "format"]
timeout = 120
enabled = false
depends_on = ["install"]

[dependencies]
required = []
optional = []

[workspace]
exclude = []
"#.to_string(),
        ProjectType::Swift => {
            let is_spm = root.join("Package.swift").exists();
            if is_spm {
                r#"# local-ci configuration for Swift (SPM) project
# See: https://github.com/stevedores-org/local-ci

[cache]
skip_dirs = [".git", ".github", "scripts", ".claude", ".build", ".swiftpm"]
include_patterns = ["*.swift", "Package.swift", "Package.resolved"]

[stages.fmt]
command = ["swift-format", "lint", "--recursive", "."]
fix_command = ["swift-format", "format", "--in-place", "--recursive", "."]
timeout = 120
enabled = true

[stages.build]
command = ["swift", "build"]
timeout = 600
enabled = true

[stages.test]
command = ["swift", "test"]
timeout = 1200
enabled = true

[dependencies]
required = ["swift-format"]
optional = ["swiftlint"]

[workspace]
exclude = []
"#.to_string()
            } else {
                r#"# local-ci configuration for Swift (Xcode) project
# See: https://github.com/stevedores-org/local-ci

[cache]
skip_dirs = [".git", ".github", "scripts", ".claude", "DerivedData", "Pods"]
include_patterns = ["*.swift", "*.xcconfig", "project.pbxproj"]

[stages.fmt]
command = ["swift-format", "lint", "--recursive", "."]
fix_command = ["swift-format", "format", "--in-place", "--recursive", "."]
timeout = 120
enabled = true

[stages.build]
command = ["xcodebuild", "-scheme", "Placeholder", "build"]
timeout = 600
enabled = true

[stages.test]
command = ["xcodebuild", "test", "-scheme", "Placeholder", "-destination", "platform=macOS"]
timeout = 1200
enabled = true

[dependencies]
required = ["swift-format"]
optional = ["swiftlint"]

[workspace]
exclude = []
"#.to_string()
            }
        }
        ProjectType::Python => r#"# local-ci configuration for Python project
# See: https://github.com/stevedores-org/local-ci

[cache]
skip_dirs = [".git", ".github", "scripts", ".claude", ".pytest_cache", "__pycache__", ".mypy_cache", ".venv", "venv"]
include_patterns = ["*.py", "*.toml", "*.txt", "*.yml", "*.yaml"]

[stages.lint]
command = ["pylint", ".", "--errors-only"]
timeout = 300
enabled = false

[stages.format]
command = ["black", "--check", "."]
fix_command = ["black", "."]
timeout = 120
enabled = false

[stages.test]
command = ["pytest"]
timeout = 600
enabled = false

[dependencies]
optional = []

[workspace]
exclude = []
"#.to_string(),
        ProjectType::Go => r#"# local-ci configuration for Go project
# See: https://github.com/stevedores-org/local-ci

[cache]
skip_dirs = [".git", ".github", "scripts", ".claude", "vendor"]
include_patterns = ["*.go", "go.mod", "go.sum"]

[stages.fmt]
command = ["go", "fmt", "./..."]
fix_command = ["go", "fmt", "./..."]
timeout = 120
enabled = false

[stages.vet]
command = ["go", "vet", "./..."]
timeout = 300
enabled = false

[stages.test]
command = ["go", "test", "./..."]
timeout = 600
enabled = false

[dependencies]
optional = []

[workspace]
exclude = []
"#.to_string(),
        ProjectType::Java => r#"# local-ci configuration for Java project
# See: https://github.com/stevedores-org/local-ci

[cache]
skip_dirs = [".git", ".github", "scripts", ".claude", "target", "build"]
include_patterns = ["*.java", "pom.xml", "build.gradle"]

[stages.build]
command = ["mvn", "clean", "compile"]
timeout = 600
enabled = false

[stages.test]
command = ["mvn", "test"]
timeout = 900
enabled = false

[dependencies]
optional = []

[workspace]
exclude = []
"#.to_string(),
        ProjectType::Generic => r#"# local-ci configuration (Generic)
# See: https://github.com/stevedores-org/local-ci

[cache]
skip_dirs = [".git", ".github", "scripts", ".claude", "node_modules", "target", "build", "dist"]
include_patterns = ["*"]

[stages.placeholder]
command = ["echo", "Configure stages in .local-ci.toml"]
timeout = 60
enabled = false

[dependencies]
optional = []

[workspace]
exclude = []
"#.to_string(),
    }
}

pub fn load_config(root: &Path, remote: bool) -> Result<Config, String> {
    let project_type = detect_project_type(root);
    let default_stages = get_default_stages_for_type(project_type, root);
    let cache_patterns = get_cache_pattern_for_type(project_type);
    let skip_dirs = get_skip_dirs_for_type(project_type);

    let mut cfg = Config {
        cache: CacheConfig {
            skip_dirs,
            include_patterns: cache_patterns,
        },
        stages: default_stages,
        dependencies: DepsConfig::default(),
        workspace: WorkspaceConfig::default(),
        profiles: HashMap::new(),
        hosts: HashMap::new(),
        ssh_defaults: RemoteSSHDefaults::default(),
    };

    // Load local TOML
    let config_path = root.join(".local-ci.toml");
    if config_path.exists() {
        let data = fs::read_to_string(&config_path)
            .map_err(|e| format!("Failed to read .local-ci.toml: {}", e))?;
        let local_cfg: Config =
            toml::from_str(&data).map_err(|e| format!("Failed to parse .local-ci.toml: {}", e))?;

        // Merge stage map
        for (name, stage) in local_cfg.stages {
            // Respect Command / Cmd parsing aliases
            if stage.command.is_some() {
                // If a stage is defined locally, merge it
                cfg.stages.insert(name, stage);
            } else if let Some(existing) = cfg.stages.get_mut(&name) {
                // Keep default command if none was specified, but update other settings
                if let Some(fix_cmd) = stage.fix_command {
                    existing.fix_command = Some(fix_cmd);
                }
                existing.check = stage.check;
                if stage.timeout > 0 {
                    existing.timeout = stage.timeout;
                }
                existing.enabled = stage.enabled;
                if !stage.depends_on.is_empty() {
                    existing.depends_on = stage.depends_on;
                }
                if !stage.watch.is_empty() {
                    existing.watch = stage.watch;
                }
            } else {
                // Omitted command in user-defined custom stage means it is invalid unless we have it in default
                cfg.stages.insert(name, stage);
            }
        }

        // Merge cache configs
        if !local_cfg.cache.skip_dirs.is_empty() {
            cfg.cache.skip_dirs = local_cfg.cache.skip_dirs;
        }
        if !local_cfg.cache.include_patterns.is_empty() {
            cfg.cache.include_patterns = local_cfg.cache.include_patterns;
        }

        // Merge workspaces
        if !local_cfg.workspace.exclude.is_empty() {
            cfg.workspace.exclude = local_cfg.workspace.exclude;
        }

        // Merge profiles
        for (name, p) in local_cfg.profiles {
            cfg.profiles.insert(name, p);
        }

        // Merge dependencies
        if !local_cfg.dependencies.required.is_empty() {
            cfg.dependencies.required = local_cfg.dependencies.required;
        }
        if !local_cfg.dependencies.optional.is_empty() {
            cfg.dependencies.optional = local_cfg.dependencies.optional;
        }
    }

    if remote {
        let remote_path = root.join(".local-ci-remote.toml");
        if remote_path.exists() {
            let r_data = fs::read_to_string(&remote_path)
                .map_err(|e| format!("Failed to read .local-ci-remote.toml: {}", e))?;
            let r_cfg: Config = toml::from_str(&r_data)
                .map_err(|e| format!("Failed to parse .local-ci-remote.toml: {}", e))?;

            // Merge remote stages (override local stages if specified in remote)
            for (name, stage) in r_cfg.stages {
                cfg.stages.insert(name, stage);
            }

            // Merge remote cache config if specified
            if !r_cfg.cache.skip_dirs.is_empty() {
                cfg.cache.skip_dirs = r_cfg.cache.skip_dirs;
            }
            if !r_cfg.cache.include_patterns.is_empty() {
                cfg.cache.include_patterns = r_cfg.cache.include_patterns;
            }

            // Merge remote dependencies
            if !r_cfg.dependencies.required.is_empty() {
                cfg.dependencies.required = r_cfg.dependencies.required;
            }
            if !r_cfg.dependencies.optional.is_empty() {
                cfg.dependencies.optional = r_cfg.dependencies.optional;
            }

            // Merge remote workspace
            if !r_cfg.workspace.exclude.is_empty() {
                cfg.workspace.exclude = r_cfg.workspace.exclude;
            }

            if !r_cfg.ssh_defaults.macos_user.is_empty()
                || !r_cfg.ssh_defaults.linux_spark_user.is_empty()
                || !r_cfg.ssh_defaults.windows_user.is_empty()
            {
                cfg.ssh_defaults = r_cfg.ssh_defaults;
            }

            for (name, host) in r_cfg.hosts {
                cfg.hosts.insert(name, host);
            }
        }
    }

    // Set name for all stages
    for (name, stage) in &mut cfg.stages {
        stage.name = name.clone();
    }

    Ok(cfg)
}
