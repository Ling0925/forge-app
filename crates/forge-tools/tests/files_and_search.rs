use forge_tools::{
    Tool, ToolError, ToolRequest,
    cancellation::CancellationToken,
    files::{
        EditFileRequest, EditFileTool, FileChange, ListDirTool, ReadFileRequest, ReadFileTool,
        WriteFileRequest, WriteFileTool,
    },
    search::{GlobRequest, GlobTool, GrepMatch, GrepRequest, GrepTool},
    workspace::WorkspaceRoot,
};
use sha2::{Digest, Sha256};
use std::{fs, ops::RangeInclusive, path::PathBuf};

async fn run<T: Tool>(tool: &T, input: T::Request) -> Result<T::Output, ToolError> {
    let request = ToolRequest {
        call: "file-test".into(),
        input,
    };
    let result = tool.execute(request).await?;
    assert_eq!(result.call, "file-test".into());
    Ok(result.output)
}

fn hash(content: impl AsRef<[u8]>) -> String {
    format!("{:x}", Sha256::digest(content.as_ref()))
}

fn glob(path: impl Into<PathBuf>, pattern: &str) -> GlobRequest {
    GlobRequest {
        path: path.into(),
        pattern: pattern.into(),
        cancellation: CancellationToken::default(),
    }
}

fn grep(path: impl Into<PathBuf>, pattern: &str) -> GrepRequest {
    GrepRequest {
        path: path.into(),
        pattern: pattern.into(),
        cancellation: CancellationToken::default(),
    }
}

#[test]
fn tool_names_and_approval_metadata() {
    let directory = tempfile::tempdir().unwrap();
    let root = WorkspaceRoot::new(directory.path()).unwrap();
    fn check(tool: impl Tool, name: &str, writes: bool) {
        assert_eq!(tool.name(), name);
        assert_eq!(tool.has_side_effects(), writes);
    }
    check(ReadFileTool::new(root.clone()), "read_file", false);
    check(WriteFileTool::new(root.clone()), "write_file", true);
    check(EditFileTool::new(root.clone()), "edit_file", true);
    check(ListDirTool::new(root.clone()), "list_dir", false);
    check(GlobTool::new(root.clone()), "glob", false);
    check(GrepTool::new(root), "grep", false);
    #[cfg(unix)]
    check(
        forge_tools::command::CommandTool::new(std::sync::Arc::new(
            forge_tools::clock::FakeClock::default(),
        )),
        "command",
        true,
    );
}

#[tokio::test]
async fn read_file_preserves_content_and_inclusive_line_ranges() {
    let directory = tempfile::tempdir().unwrap();
    let content = "α\r\n\nthird\nlast";
    fs::write(directory.path().join("text"), content).unwrap();
    let tool = ReadFileTool::new(WorkspaceRoot::new(directory.path()).unwrap());
    for (lines, expected) in [
        (None, content),
        (Some(1..=1), "α\r\n"),
        (Some(2..=3), "\nthird\n"),
        (Some(4..=10), "last"),
        (Some(5..=8), ""),
    ] {
        let output = run(
            &tool,
            ReadFileRequest {
                path: "text".into(),
                lines,
            },
        )
        .await
        .unwrap();
        assert_eq!(output, expected);
    }
    fs::write(directory.path().join("text"), "").unwrap();
    assert_eq!(
        run(
            &tool,
            ReadFileRequest {
                path: "text".into(),
                lines: Some(1..=1),
            }
        )
        .await
        .unwrap(),
        ""
    );
}

#[tokio::test]
async fn read_file_rejects_invalid_line_ranges() {
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join("text"), "text").unwrap();
    let tool = ReadFileTool::new(WorkspaceRoot::new(directory.path()).unwrap());
    for lines in [0..=1, RangeInclusive::new(3, 2)] {
        assert!(matches!(
            run(
                &tool,
                ReadFileRequest {
                    path: "text".into(),
                    lines: Some(lines),
                }
            )
            .await,
            Err(ToolError::InvalidArguments(_))
        ));
    }
}

#[tokio::test]
async fn write_file_records_create_and_overwrite_hashes() {
    let directory = tempfile::tempdir().unwrap();
    fs::create_dir(directory.path().join("sub")).unwrap();
    let tool = WriteFileTool::new(WorkspaceRoot::new(directory.path()).unwrap());
    let mut before = None;
    for content in ["hello\n", "updated\r\n", ""] {
        let change = run(
            &tool,
            WriteFileRequest {
                path: "sub/file".into(),
                content: content.into(),
            },
        )
        .await
        .unwrap();
        assert_eq!(
            change,
            FileChange {
                path: "sub/file".into(),
                before_sha256: before,
                after_sha256: hash(content),
            }
        );
        assert_eq!(
            fs::read_to_string(directory.path().join("sub/file")).unwrap(),
            content
        );
        before = Some(hash(content));
    }
}

#[tokio::test]
async fn write_file_hashes_original_bytes() {
    let directory = tempfile::tempdir().unwrap();
    let before = b"binary\0\xff";
    fs::write(directory.path().join("file"), before).unwrap();
    let change = run(
        &WriteFileTool::new(WorkspaceRoot::new(directory.path()).unwrap()),
        WriteFileRequest {
            path: "file".into(),
            content: "text".into(),
        },
    )
    .await
    .unwrap();
    assert_eq!(change.before_sha256, Some(hash(before)));
    assert_eq!(change.after_sha256, hash("text"));
}

#[tokio::test]
async fn edit_file_replaces_exact_multiline_text_and_records_hashes() {
    let directory = tempfile::tempdir().unwrap();
    let before = "α [a.*]\nold\r\ntail\n";
    fs::write(directory.path().join("file"), before).unwrap();
    let change = run(
        &EditFileTool::new(WorkspaceRoot::new(directory.path()).unwrap()),
        EditFileRequest {
            path: "file".into(),
            old_string: "[a.*]\nold".into(),
            new_string: "new".into(),
        },
    )
    .await
    .unwrap();
    let after = "α new\r\ntail\n";
    assert_eq!(
        fs::read_to_string(directory.path().join("file")).unwrap(),
        after
    );
    assert_eq!(
        change,
        FileChange {
            path: "file".into(),
            before_sha256: Some(hash(before)),
            after_sha256: hash(after),
        }
    );
}

#[tokio::test]
async fn edit_file_rejects_missing_repeated_and_overlapping_matches_without_writing() {
    let directory = tempfile::tempdir().unwrap();
    let tool = EditFileTool::new(WorkspaceRoot::new(directory.path()).unwrap());
    for (before, old_string, message) in [
        ("original", "missing", "old string not found"),
        ("same same", "same", "old string is not unique"),
        ("banana", "ana", "old string is not unique"),
        ("original", "", "old string is not unique"),
    ] {
        fs::write(directory.path().join("file"), before).unwrap();
        let error = run(
            &tool,
            EditFileRequest {
                path: "file".into(),
                old_string: old_string.into(),
                new_string: "changed".into(),
            },
        )
        .await
        .unwrap_err();
        assert_eq!(error.to_string(), message);
        assert_eq!(
            fs::read_to_string(directory.path().join("file")).unwrap(),
            before
        );
    }
}

#[tokio::test]
async fn list_dir_returns_sorted_immediate_entries_with_types() {
    let directory = tempfile::tempdir().unwrap();
    fs::create_dir_all(directory.path().join("sub/nested")).unwrap();
    fs::write(directory.path().join("sub/nested/deep"), "deep").unwrap();
    fs::write(directory.path().join("sub/file"), "file").unwrap();
    fs::write(directory.path().join("sub/.hidden"), "hidden").unwrap();
    let tool = ListDirTool::new(WorkspaceRoot::new(directory.path()).unwrap());
    let entries = run(&tool, "sub".into()).await.unwrap();
    assert_eq!(
        entries.iter().map(|entry| &entry.path).collect::<Vec<_>>(),
        [
            &PathBuf::from("sub/.hidden"),
            &"sub/file".into(),
            &"sub/nested".into()
        ]
    );
    assert!(entries[0].file_type.is_file());
    assert!(entries[1].file_type.is_file());
    assert!(entries[2].file_type.is_dir());
}

#[tokio::test]
async fn glob_matches_workspace_relative_paths_recursively() {
    let directory = tempfile::tempdir().unwrap();
    fs::create_dir_all(directory.path().join("src/nested")).unwrap();
    for path in [
        "main.rs",
        ".hidden.rs",
        "src/lib.rs",
        "src/nested/mod.rs",
        "src/lib.ts",
    ] {
        fs::write(directory.path().join(path), "").unwrap();
    }
    let tool = GlobTool::new(WorkspaceRoot::new(directory.path()).unwrap());
    for (path, pattern, expected) in [
        (
            ".",
            "**/*.rs",
            vec!["main.rs", "src/lib.rs", "src/nested/mod.rs"],
        ),
        (".", "*.rs", vec!["main.rs"]),
        ("src", "src/*.{rs,ts}", vec!["src/lib.rs", "src/lib.ts"]),
        ("src/lib.rs", "**/*.rs", vec!["src/lib.rs"]),
        (".", "**/*.absent", vec![]),
    ] {
        assert_eq!(
            run(&tool, glob(path, pattern)).await.unwrap(),
            expected.into_iter().map(PathBuf::from).collect::<Vec<_>>()
        );
    }
}

#[tokio::test]
async fn grep_returns_matching_lines_with_paths_and_one_based_numbers() {
    let directory = tempfile::tempdir().unwrap();
    fs::create_dir(directory.path().join("sub")).unwrap();
    fs::write(directory.path().join("a"), "miss\nhit 12 hit 13\r\nhit 9\n").unwrap();
    fs::write(directory.path().join("sub/b"), "hit 42").unwrap();
    fs::write(directory.path().join(".hidden"), "hit 99\n").unwrap();
    let tool = GrepTool::new(WorkspaceRoot::new(directory.path()).unwrap());
    let matches = run(&tool, grep(".", r"hit \d{2}")).await.unwrap();
    assert_eq!(
        matches,
        vec![
            GrepMatch {
                path: "a".into(),
                line_number: 2,
                line: "hit 12 hit 13".into()
            },
            GrepMatch {
                path: "sub/b".into(),
                line_number: 1,
                line: "hit 42".into()
            },
        ]
    );
    assert_eq!(run(&tool, grep("sub", "^hit 42$")).await.unwrap().len(), 1);
    assert_eq!(
        run(&tool, grep("a", "^hit 9$")).await.unwrap()[0].line_number,
        3
    );
    assert!(run(&tool, grep(".", "absent")).await.unwrap().is_empty());
}

#[tokio::test]
async fn searches_skip_ignored_files_and_hidden_directories() {
    let directory = tempfile::tempdir().unwrap();
    fs::create_dir(directory.path().join(".hidden")).unwrap();
    fs::create_dir(directory.path().join(".git")).unwrap();
    fs::write(directory.path().join(".ignore"), "ignored.txt\n").unwrap();
    for path in [
        "visible.txt",
        "ignored.txt",
        ".hidden/file.txt",
        ".git/file.txt",
    ] {
        fs::write(directory.path().join(path), "match\n").unwrap();
    }
    let root = WorkspaceRoot::new(directory.path()).unwrap();
    assert_eq!(
        run(&GlobTool::new(root.clone()), glob(".", "**/*.txt"))
            .await
            .unwrap(),
        vec![PathBuf::from("visible.txt")]
    );
    assert_eq!(
        run(&GrepTool::new(root), grep(".", "match")).await.unwrap(),
        vec![GrepMatch {
            path: "visible.txt".into(),
            line_number: 1,
            line: "match".into()
        }]
    );
}

#[tokio::test]
async fn grep_searches_non_utf8_content_without_aborting_the_tree() {
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join("bytes"), b"\xff hit\n").unwrap();
    fs::write(directory.path().join("text"), "αβ\n").unwrap();
    let tool = GrepTool::new(WorkspaceRoot::new(directory.path()).unwrap());
    let matches = run(&tool, grep(".", "hit|αβ")).await.unwrap();
    assert_eq!(matches.len(), 2);
    assert_eq!(matches[0].line, "\u{fffd} hit");
    assert_eq!(matches[1].line, "αβ");
}

#[tokio::test]
async fn search_rejects_invalid_patterns() {
    let directory = tempfile::tempdir().unwrap();
    let root = WorkspaceRoot::new(directory.path()).unwrap();
    assert!(matches!(
        run(&GlobTool::new(root.clone()), glob(".", "[")).await,
        Err(ToolError::InvalidArguments(_))
    ));
    assert!(matches!(
        run(&GrepTool::new(root), grep(".", "[")).await,
        Err(ToolError::InvalidArguments(_))
    ));
}

#[tokio::test]
async fn grep_and_glob_return_cancelled_for_a_cancelled_request() {
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join("file"), "match").unwrap();
    let root = WorkspaceRoot::new(directory.path()).unwrap();
    let cancellation = CancellationToken::default();
    let glob_input = GlobRequest {
        cancellation: cancellation.clone(),
        ..glob(".", "**/*")
    };
    let grep_input = GrepRequest {
        cancellation: cancellation.clone(),
        ..grep(".", "match")
    };
    cancellation.cancel();
    assert!(matches!(
        run(&GlobTool::new(root.clone()), glob_input).await,
        Err(ToolError::Cancelled)
    ));
    assert!(matches!(
        run(&GrepTool::new(root), grep_input).await,
        Err(ToolError::Cancelled)
    ));
}

async fn assert_path_rejected(root: &WorkspaceRoot, path: PathBuf) {
    let errors = [
        run(
            &ReadFileTool::new(root.clone()),
            ReadFileRequest {
                path: path.clone(),
                lines: None,
            },
        )
        .await
        .unwrap_err(),
        run(
            &WriteFileTool::new(root.clone()),
            WriteFileRequest {
                path: path.clone(),
                content: "changed".into(),
            },
        )
        .await
        .unwrap_err(),
        run(
            &EditFileTool::new(root.clone()),
            EditFileRequest {
                path: path.clone(),
                old_string: "original".into(),
                new_string: "changed".into(),
            },
        )
        .await
        .unwrap_err(),
        run(&ListDirTool::new(root.clone()), path.clone())
            .await
            .unwrap_err(),
        run(&GlobTool::new(root.clone()), glob(&path, "**/*"))
            .await
            .unwrap_err(),
        run(&GrepTool::new(root.clone()), grep(&path, "original"))
            .await
            .unwrap_err(),
    ];
    for error in errors {
        assert!(
            matches!(error, ToolError::InvalidArguments(_)),
            "{path:?}: {error}"
        );
    }
}

#[tokio::test]
async fn every_tool_rejects_parent_and_absolute_path_escapes() {
    let directory = tempfile::tempdir().unwrap();
    fs::create_dir(directory.path().join("root")).unwrap();
    fs::create_dir(directory.path().join("root/sub")).unwrap();
    fs::write(directory.path().join("outside"), "original").unwrap();
    let root = WorkspaceRoot::new(directory.path().join("root")).unwrap();
    for path in [
        PathBuf::from("../outside"),
        "sub/../../outside".into(),
        directory.path().join("outside"),
        directory.path().join("root/../outside"),
        directory.path().join("root-sibling/missing"),
    ] {
        assert_path_rejected(&root, path).await;
    }
    assert_eq!(
        fs::read_to_string(directory.path().join("outside")).unwrap(),
        "original"
    );
}

#[tokio::test]
async fn paths_inside_root_allow_absolute_paths_and_parent_components() {
    let directory = tempfile::tempdir().unwrap();
    fs::create_dir(directory.path().join("sub")).unwrap();
    fs::write(directory.path().join("file"), "content").unwrap();
    let root = WorkspaceRoot::new(directory.path()).unwrap();
    for path in [
        "sub/../file".into(),
        fs::canonicalize(directory.path()).unwrap().join("file"),
    ] {
        assert_eq!(
            run(
                &ReadFileTool::new(root.clone()),
                ReadFileRequest { path, lines: None }
            )
            .await
            .unwrap(),
            "content"
        );
    }
}

#[tokio::test]
async fn missing_files_error_on_read_and_missing_parents_are_created_on_write() {
    let directory = tempfile::tempdir().unwrap();
    let root = WorkspaceRoot::new(directory.path()).unwrap();
    assert!(matches!(
        run(&ReadFileTool::new(root.clone()), ReadFileRequest { path: "missing".into(), lines: None }).await,
        Err(ToolError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound
    ));
    let change = run(
        &WriteFileTool::new(root),
        WriteFileRequest {
            path: "missing/file".into(),
            content: "text".into(),
        },
    )
    .await
    .unwrap();
    assert_eq!(change.before_sha256, None);
    assert_eq!(
        fs::read_to_string(directory.path().join("missing/file")).unwrap(),
        "text"
    );
}

#[tokio::test]
async fn write_file_creates_nested_parents_inside_root() {
    let directory = tempfile::tempdir().unwrap();
    let root = WorkspaceRoot::new(directory.path()).unwrap();
    let change = run(
        &WriteFileTool::new(root),
        WriteFileRequest {
            path: "one/unused/../two/three/file".into(),
            content: "nested".into(),
        },
    )
    .await
    .unwrap();
    assert_eq!(
        change,
        FileChange {
            path: "one/two/three/file".into(),
            before_sha256: None,
            after_sha256: hash("nested"),
        }
    );
    assert_eq!(
        fs::read_to_string(directory.path().join("one/two/three/file")).unwrap(),
        "nested"
    );
    assert!(!directory.path().join("one/unused").exists());
}

#[tokio::test]
async fn write_file_rejects_parent_escapes_before_creating_directories() {
    let directory = tempfile::tempdir().unwrap();
    fs::create_dir(directory.path().join("root")).unwrap();
    let tool = WriteFileTool::new(WorkspaceRoot::new(directory.path().join("root")).unwrap());
    for path in ["new/../../outside/file", "new/deep/../../../outside/file"] {
        assert!(matches!(
            run(
                &tool,
                WriteFileRequest {
                    path: path.into(),
                    content: "escaped".into()
                }
            )
            .await,
            Err(ToolError::InvalidArguments(_))
        ));
        assert!(!directory.path().join("root/new").exists());
        assert!(!directory.path().join("outside").exists());
    }
}

#[cfg(unix)]
mod symlinks {
    use super::*;
    use std::os::unix::fs::symlink;

    #[tokio::test]
    async fn absolute_paths_through_a_symlinked_workspace_root_are_accepted() {
        let directory = tempfile::tempdir().unwrap();
        fs::create_dir(directory.path().join("real")).unwrap();
        symlink("real", directory.path().join("workspace")).unwrap();
        let root = WorkspaceRoot::new(directory.path().join("workspace")).unwrap();
        let path = directory.path().join("workspace/file");
        let change = run(
            &WriteFileTool::new(root.clone()),
            WriteFileRequest {
                path: path.clone(),
                content: "content".into(),
            },
        )
        .await
        .unwrap();
        assert_eq!(change.path, PathBuf::from("file"));
        assert_eq!(
            run(
                &ReadFileTool::new(root),
                ReadFileRequest { path, lines: None }
            )
            .await
            .unwrap(),
            "content"
        );
    }

    #[tokio::test]
    async fn write_file_follows_in_root_dangling_symlinks_and_rejects_loops() {
        let directory = tempfile::tempdir().unwrap();
        symlink("sub/new", directory.path().join("link")).unwrap();
        symlink("link", directory.path().join("chain")).unwrap();
        symlink("cycle", directory.path().join("cycle")).unwrap();
        symlink(
            "missing/../hidden-cycle",
            directory.path().join("hidden-cycle"),
        )
        .unwrap();
        let tool = WriteFileTool::new(WorkspaceRoot::new(directory.path()).unwrap());
        let change = run(
            &tool,
            WriteFileRequest {
                path: "chain".into(),
                content: "created".into(),
            },
        )
        .await
        .unwrap();
        assert_eq!(change.path, PathBuf::from("sub/new"));
        assert_eq!(change.before_sha256, None);
        assert_eq!(change.after_sha256, hash("created"));
        assert_eq!(
            fs::read_to_string(directory.path().join("sub/new")).unwrap(),
            "created"
        );
        assert!(
            fs::symlink_metadata(directory.path().join("link"))
                .unwrap()
                .is_symlink()
        );
        assert!(matches!(
            run(
                &tool,
                WriteFileRequest {
                    path: "cycle".into(),
                    content: "content".into()
                }
            )
            .await,
            Err(ToolError::Io(_))
        ));
        assert!(matches!(
            run(
                &tool,
                WriteFileRequest {
                    path: "hidden-cycle".into(),
                    content: "content".into()
                }
            )
            .await,
            Err(ToolError::Io(_))
        ));
        assert!(!directory.path().join("missing").exists());
    }

    #[tokio::test]
    async fn write_file_rejects_external_symlink_ancestors_before_creating_directories() {
        let directory = tempfile::tempdir().unwrap();
        fs::create_dir(directory.path().join("root")).unwrap();
        fs::create_dir(directory.path().join("outside")).unwrap();
        symlink("../outside", directory.path().join("root/external")).unwrap();
        symlink("../absent", directory.path().join("root/dangling")).unwrap();
        let tool = WriteFileTool::new(WorkspaceRoot::new(directory.path().join("root")).unwrap());
        for path in [
            "external/new/deep/file",
            "new/../external/deep/file",
            "dangling/new/deep/file",
        ] {
            assert!(matches!(
                run(
                    &tool,
                    WriteFileRequest {
                        path: path.into(),
                        content: "escaped".into()
                    }
                )
                .await,
                Err(ToolError::InvalidArguments(_))
            ));
            assert!(!directory.path().join("root/new").exists());
            assert_eq!(
                fs::read_dir(directory.path().join("outside"))
                    .unwrap()
                    .count(),
                0
            );
            assert!(!directory.path().join("absent").exists());
        }
    }

    #[tokio::test]
    async fn every_tool_rejects_external_symlinks_and_symlinked_parents() {
        let directory = tempfile::tempdir().unwrap();
        fs::create_dir(directory.path().join("root")).unwrap();
        fs::create_dir(directory.path().join("outside")).unwrap();
        fs::write(directory.path().join("outside/file"), "original").unwrap();
        symlink("../outside/file", directory.path().join("root/link")).unwrap();
        symlink("../outside", directory.path().join("root/parent")).unwrap();
        let root = WorkspaceRoot::new(directory.path().join("root")).unwrap();
        for path in [
            "link",
            "parent",
            "parent/file",
            "parent/new",
            "parent/../root",
        ] {
            assert_path_rejected(&root, path.into()).await;
        }
        assert_eq!(
            fs::read_to_string(directory.path().join("outside/file")).unwrap(),
            "original"
        );
        assert!(!directory.path().join("outside/new").exists());
    }

    #[tokio::test]
    async fn write_file_does_not_follow_dangling_external_symlinks() {
        let directory = tempfile::tempdir().unwrap();
        fs::create_dir(directory.path().join("root")).unwrap();
        symlink("../missing", directory.path().join("root/link")).unwrap();
        let result = run(
            &WriteFileTool::new(WorkspaceRoot::new(directory.path().join("root")).unwrap()),
            WriteFileRequest {
                path: "link".into(),
                content: "escaped".into(),
            },
        )
        .await;
        assert!(result.is_err());
        assert!(!directory.path().join("missing").exists());
        assert!(
            fs::symlink_metadata(directory.path().join("root/link"))
                .unwrap()
                .is_symlink()
        );
    }

    #[tokio::test]
    async fn in_root_symlinks_resolve_to_target_paths_for_reads_writes_and_edits() {
        let directory = tempfile::tempdir().unwrap();
        fs::create_dir(directory.path().join("real")).unwrap();
        fs::write(directory.path().join("real/file"), "original").unwrap();
        symlink("real", directory.path().join("alias")).unwrap();
        symlink("real/file", directory.path().join("link")).unwrap();
        let root = WorkspaceRoot::new(directory.path()).unwrap();
        assert_eq!(
            run(
                &ReadFileTool::new(root.clone()),
                ReadFileRequest {
                    path: "alias/file".into(),
                    lines: None
                }
            )
            .await
            .unwrap(),
            "original"
        );
        let change = run(
            &EditFileTool::new(root.clone()),
            EditFileRequest {
                path: "link".into(),
                old_string: "original".into(),
                new_string: "edited".into(),
            },
        )
        .await
        .unwrap();
        assert_eq!(change.path, PathBuf::from("real/file"));
        assert_eq!(change.before_sha256, Some(hash("original")));
        assert_eq!(change.after_sha256, hash("edited"));
        let change = run(
            &WriteFileTool::new(root.clone()),
            WriteFileRequest {
                path: "alias/new".into(),
                content: "created".into(),
            },
        )
        .await
        .unwrap();
        assert_eq!(change.path, PathBuf::from("real/new"));
        assert_eq!(change.before_sha256, None);
        assert_eq!(
            fs::read_to_string(directory.path().join("real/new")).unwrap(),
            "created"
        );
        assert_eq!(
            run(&ListDirTool::new(root.clone()), "alias".into())
                .await
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            run(&GlobTool::new(root.clone()), glob("alias", "real/*"))
                .await
                .unwrap(),
            vec![PathBuf::from("real/file"), "real/new".into()]
        );
        assert_eq!(
            run(&GrepTool::new(root), grep("alias", "edited"))
                .await
                .unwrap()[0]
                .path,
            PathBuf::from("real/file")
        );
        assert!(
            fs::symlink_metadata(directory.path().join("link"))
                .unwrap()
                .is_symlink()
        );
    }

    #[tokio::test]
    async fn search_walks_do_not_follow_symlink_entries() {
        let directory = tempfile::tempdir().unwrap();
        fs::create_dir(directory.path().join("root")).unwrap();
        fs::create_dir(directory.path().join("outside")).unwrap();
        fs::write(directory.path().join("outside/file"), "outside").unwrap();
        fs::write(directory.path().join("root/file"), "inside").unwrap();
        symlink("../outside", directory.path().join("root/external")).unwrap();
        symlink(".", directory.path().join("root/loop")).unwrap();
        let root = WorkspaceRoot::new(directory.path().join("root")).unwrap();
        assert_eq!(
            run(&GlobTool::new(root.clone()), glob(".", "**/*"))
                .await
                .unwrap(),
            vec![PathBuf::from("file")]
        );
        assert_eq!(
            run(&GrepTool::new(root.clone()), grep(".", "."))
                .await
                .unwrap(),
            vec![GrepMatch {
                path: "file".into(),
                line_number: 1,
                line: "inside".into()
            }]
        );
        let entries = run(&ListDirTool::new(root), ".".into()).await.unwrap();
        assert!(entries[0].file_type.is_symlink());
        assert!(entries[2].file_type.is_symlink());
    }
}

#[cfg(feature = "failpoints")]
#[tokio::test]
async fn writes_and_edits_hit_the_failpoint_after_mutation() {
    use forge_tools::failpoints::{
        Failpoints, InjectedCrash, TOOL_AFTER_SIDE_EFFECT_BEFORE_COMMIT,
    };
    let directory = tempfile::tempdir().unwrap();
    let root = WorkspaceRoot::new(directory.path()).unwrap();
    let mut writer = WriteFileTool::new(root.clone());
    writer.failpoints = Failpoints::new([TOOL_AFTER_SIDE_EFFECT_BEFORE_COMMIT]);
    let write = tokio::spawn(async move {
        run(
            &writer,
            WriteFileRequest {
                path: "file".into(),
                content: "created".into(),
            },
        )
        .await
    })
    .await
    .unwrap_err();
    assert_eq!(
        write
            .into_panic()
            .downcast::<InjectedCrash>()
            .unwrap()
            .point,
        TOOL_AFTER_SIDE_EFFECT_BEFORE_COMMIT
    );
    assert_eq!(
        fs::read_to_string(directory.path().join("file")).unwrap(),
        "created"
    );
    let mut editor = EditFileTool::new(root);
    editor.failpoints = Failpoints::new([TOOL_AFTER_SIDE_EFFECT_BEFORE_COMMIT]);
    let edit = tokio::spawn(async move {
        run(
            &editor,
            EditFileRequest {
                path: "file".into(),
                old_string: "created".into(),
                new_string: "edited".into(),
            },
        )
        .await
    })
    .await
    .unwrap_err();
    assert_eq!(
        edit.into_panic().downcast::<InjectedCrash>().unwrap().point,
        TOOL_AFTER_SIDE_EFFECT_BEFORE_COMMIT
    );
    assert_eq!(
        fs::read_to_string(directory.path().join("file")).unwrap(),
        "edited"
    );
}
