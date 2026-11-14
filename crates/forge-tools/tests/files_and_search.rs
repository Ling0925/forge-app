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
use std::{
    ffi::OsString,
    fs,
    ops::RangeInclusive,
    path::{Path, PathBuf},
};

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

fn entry_names(path: &Path) -> Vec<OsString> {
    let mut names = fs::read_dir(path)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<Vec<_>>();
    names.sort();
    names
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
    for (content, lines_added, lines_removed) in
        [("hello\n", 1, 0), ("updated\r\n", 1, 1), ("", 0, 1)]
    {
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
                lines_added,
                lines_removed,
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
async fn write_file_counts_lines_in_new_files() {
    let directory = tempfile::tempdir().unwrap();
    let tool = WriteFileTool::new(WorkspaceRoot::new(directory.path()).unwrap());
    for (path, content, added) in [
        ("empty", "", 0),
        ("blank", "\n", 1),
        ("terminated", "first\nlast\n", 2),
        ("unterminated", "first\nlast", 2),
    ] {
        let change = run(
            &tool,
            WriteFileRequest {
                path: path.into(),
                content: content.into(),
            },
        )
        .await
        .unwrap();
        assert_eq!(change.before_sha256, None);
        assert_eq!((change.lines_added, change.lines_removed), (added, 0));
    }
}

#[tokio::test]
async fn write_file_counts_only_changed_lines_when_overwriting() {
    let directory = tempfile::tempdir().unwrap();
    fs::write(
        directory.path().join("file"),
        "head\nremove one\nmiddle\nremove two\ntail\n",
    )
    .unwrap();
    let change = run(
        &WriteFileTool::new(WorkspaceRoot::new(directory.path()).unwrap()),
        WriteFileRequest {
            path: "file".into(),
            content: "head\ninsert one\ninsert two\nmiddle\ntail\n".into(),
        },
    )
    .await
    .unwrap();
    assert_eq!((change.lines_added, change.lines_removed), (2, 2));
}

#[tokio::test]
async fn unchanged_writes_have_zero_line_changes() {
    let directory = tempfile::tempdir().unwrap();
    let tool = WriteFileTool::new(WorkspaceRoot::new(directory.path()).unwrap());
    for content in ["", "same\nlines\n", "same\nlast", "same\r\nlines\r\n"] {
        fs::write(directory.path().join("file"), content).unwrap();
        let change = run(
            &tool,
            WriteFileRequest {
                path: "file".into(),
                content: content.into(),
            },
        )
        .await
        .unwrap();
        assert_eq!((change.lines_added, change.lines_removed), (0, 0));
        assert_eq!(change.before_sha256, Some(change.after_sha256));
    }
}

#[tokio::test]
async fn changing_only_the_final_newline_counts_as_one_removed_and_added_line() {
    let directory = tempfile::tempdir().unwrap();
    let tool = WriteFileTool::new(WorkspaceRoot::new(directory.path()).unwrap());
    for (before, after) in [
        ("first\nlast", "first\nlast\n"),
        ("first\nlast\n", "first\nlast"),
    ] {
        fs::write(directory.path().join("file"), before).unwrap();
        let change = run(
            &tool,
            WriteFileRequest {
                path: "file".into(),
                content: after.into(),
            },
        )
        .await
        .unwrap();
        assert_eq!((change.lines_added, change.lines_removed), (1, 1));
    }
}

#[tokio::test]
async fn crlf_lines_are_compared_with_their_full_line_endings() {
    let directory = tempfile::tempdir().unwrap();
    let tool = WriteFileTool::new(WorkspaceRoot::new(directory.path()).unwrap());
    for (before, after, added, removed) in [
        ("keep\r\nold\r\n", "keep\r\nnew\r\nextra\r\n", 2, 1),
        ("same\r\n", "same\n", 1, 1),
        ("same\r\n", "same", 1, 1),
    ] {
        fs::write(directory.path().join("file"), before).unwrap();
        let change = run(
            &tool,
            WriteFileRequest {
                path: "file".into(),
                content: after.into(),
            },
        )
        .await
        .unwrap();
        assert_eq!((change.lines_added, change.lines_removed), (added, removed));
    }
}

#[tokio::test]
async fn write_file_hashes_original_bytes() {
    let directory = tempfile::tempdir().unwrap();
    let before = b"binary\0\xff";
    let after = "binary\0\u{fffd}";
    fs::write(directory.path().join("file"), before).unwrap();
    let change = run(
        &WriteFileTool::new(WorkspaceRoot::new(directory.path()).unwrap()),
        WriteFileRequest {
            path: "file".into(),
            content: after.into(),
        },
    )
    .await
    .unwrap();
    assert_eq!(change.before_sha256, Some(hash(before)));
    assert_eq!(change.after_sha256, hash(after));
    assert_eq!((change.lines_added, change.lines_removed), (1, 1));
}

#[tokio::test]
async fn write_file_overwrite_leaves_exactly_the_new_bytes() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("file");
    let tool = WriteFileTool::new(WorkspaceRoot::new(directory.path()).unwrap());
    for (before, after) in [
        ("x".repeat(8192), "tail".to_string()),
        ("short".to_string(), "y".repeat(8192)),
        ("z".repeat(4096), "q".repeat(4096)),
        ("nonempty".to_string(), String::new()),
    ] {
        fs::write(&file, &before).unwrap();
        run(
            &tool,
            WriteFileRequest {
                path: "file".into(),
                content: after.clone(),
            },
        )
        .await
        .unwrap();
        assert_eq!(fs::read(&file).unwrap(), after.as_bytes());
    }
}

#[tokio::test]
async fn concurrent_reads_observe_only_complete_write_file_contents() {
    use std::{
        collections::HashSet,
        sync::{
            Arc, Barrier,
            atomic::{AtomicBool, AtomicU64, Ordering},
        },
    };
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("file");
    let tool = WriteFileTool::new(WorkspaceRoot::new(directory.path()).unwrap());
    let versions = [
        "initial\n".to_string(),
        "a".repeat(1 << 16),
        "b".repeat(1 << 17),
    ];
    fs::write(&file, &versions[0]).unwrap();
    let allowed = Arc::new(
        versions
            .iter()
            .map(|version| version.clone().into_bytes())
            .collect::<HashSet<_>>(),
    );
    let stop = Arc::new(AtomicBool::new(false));
    let start = Arc::new(Barrier::new(2));
    let reads = Arc::new(AtomicU64::new(0));
    let torn = Arc::new(AtomicBool::new(false));
    let reader = {
        let (allowed, stop, start, reads, torn, file) = (
            allowed.clone(),
            stop.clone(),
            start.clone(),
            reads.clone(),
            torn.clone(),
            file.clone(),
        );
        std::thread::spawn(move || {
            start.wait();
            while !stop.load(Ordering::Relaxed) && !torn.load(Ordering::Relaxed) {
                if let Ok(bytes) = fs::read(&file) {
                    reads.fetch_add(1, Ordering::Relaxed);
                    if !allowed.contains(&bytes) {
                        torn.store(true, Ordering::Relaxed);
                    }
                }
            }
        })
    };
    start.wait();
    for index in 0..40 {
        run(
            &tool,
            WriteFileRequest {
                path: "file".into(),
                content: versions[1 + index % 2].clone(),
            },
        )
        .await
        .unwrap();
    }
    stop.store(true, Ordering::Relaxed);
    reader.join().unwrap();
    assert!(reads.load(Ordering::Relaxed) > 0, "reader never ran");
    assert!(
        !torn.load(Ordering::Relaxed),
        "partial or empty file observed during overwrite"
    );
    assert_eq!(fs::read(&file).unwrap(), versions[2].as_bytes());
}

#[tokio::test]
async fn writes_leave_no_temp_files_after_success_or_failure() {
    let directory = tempfile::tempdir().unwrap();
    let tool = WriteFileTool::new(WorkspaceRoot::new(directory.path()).unwrap());
    run(
        &tool,
        WriteFileRequest {
            path: "file".into(),
            content: "content".into(),
        },
    )
    .await
    .unwrap();
    assert_eq!(entry_names(directory.path()), ["file"]);
    fs::create_dir(directory.path().join("blocked")).unwrap();
    fs::write(directory.path().join("blocked/kept"), "original").unwrap();
    for path in ["blocked", "file/child"] {
        assert!(
            run(
                &tool,
                WriteFileRequest {
                    path: path.into(),
                    content: "nope".into(),
                }
            )
            .await
            .is_err(),
            "{path}"
        );
        assert_eq!(entry_names(directory.path()), ["blocked", "file"]);
    }
    assert_eq!(
        fs::read_to_string(directory.path().join("blocked/kept")).unwrap(),
        "original"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o555)).unwrap();
        if fs::write(directory.path().join("probe"), "x").is_err() {
            assert!(
                run(
                    &tool,
                    WriteFileRequest {
                        path: "denied".into(),
                        content: "nope".into(),
                    }
                )
                .await
                .is_err()
            );
        } else {
            let _ = fs::remove_file(directory.path().join("probe"));
        }
        assert_eq!(entry_names(directory.path()), ["blocked", "file"]);
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o755)).unwrap();
    }
}

#[tokio::test]
async fn writes_racing_target_swaps_leave_no_temp_files() {
    use std::{
        sync::{
            Arc, Barrier,
            atomic::{AtomicBool, AtomicU64, Ordering},
        },
        time::{Duration, Instant},
    };
    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("target");
    let tool = WriteFileTool::new(WorkspaceRoot::new(directory.path()).unwrap());
    let stop = Arc::new(AtomicBool::new(false));
    let start = Arc::new(Barrier::new(2));
    let swaps = Arc::new(AtomicU64::new(0));
    let swapper = {
        let (stop, start, swaps, target) =
            (stop.clone(), start.clone(), swaps.clone(), target.clone());
        std::thread::spawn(move || {
            start.wait();
            while !stop.load(Ordering::Relaxed) {
                let _ = fs::remove_file(&target);
                let _ = fs::remove_dir_all(&target);
                swaps.fetch_add(fs::create_dir(&target).is_ok() as u64, Ordering::Relaxed);
                let _ = fs::remove_dir_all(&target);
                swaps.fetch_add(
                    fs::write(&target, "swapped\n").is_ok() as u64,
                    Ordering::Relaxed,
                );
            }
        })
    };
    start.wait();
    let deadline = Instant::now() + Duration::from_secs(1);
    let mut attempts = 0;
    while Instant::now() < deadline {
        attempts += 1;
        let _ = run(
            &tool,
            WriteFileRequest {
                path: "target".into(),
                content: format!("attempt {attempts}\n"),
            },
        )
        .await;
        assert!(
            entry_names(directory.path())
                .iter()
                .all(|name| name == "target"),
            "stray temp file left behind"
        );
    }
    stop.store(true, Ordering::Relaxed);
    swapper.join().unwrap();
    assert!(
        attempts > 0 && swaps.load(Ordering::Relaxed) > 0,
        "no race activity"
    );
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
            lines_added: 1,
            lines_removed: 2,
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
        [&PathBuf::from("sub/file"), &"sub/nested".into()]
    );
    assert!(entries[0].file_type.is_file());
    assert!(entries[1].file_type.is_dir());
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
        run(&ListDirTool::new(root.clone()), ".".into())
            .await
            .unwrap()
            .into_iter()
            .map(|entry| entry.path)
            .collect::<Vec<_>>(),
        vec![PathBuf::from("visible.txt")]
    );
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
async fn searches_honor_gitignore_without_a_git_directory() {
    let directory = tempfile::tempdir().unwrap();
    fs::create_dir_all(directory.path().join("sub")).unwrap();
    fs::create_dir_all(directory.path().join("build/deep")).unwrap();
    for path in [
        "keep.txt",
        "ignored.txt",
        "sub/keep.txt",
        "sub/ignored.txt",
        "build/output.txt",
        "build/deep/output.txt",
    ] {
        fs::write(directory.path().join(path), "needle\n").unwrap();
    }
    fs::write(
        directory.path().join(".gitignore"),
        "/ignored.txt\nbuild/\n",
    )
    .unwrap();
    fs::write(directory.path().join("sub/.gitignore"), "ignored.txt\n").unwrap();
    let root = WorkspaceRoot::new(directory.path()).unwrap();
    let expected = vec![PathBuf::from("keep.txt"), "sub/keep.txt".into()];
    assert_eq!(
        run(&GlobTool::new(root.clone()), glob(".", "**/*"))
            .await
            .unwrap(),
        expected
    );
    assert_eq!(
        run(&GrepTool::new(root.clone()), grep(".", "needle"))
            .await
            .unwrap()
            .into_iter()
            .map(|m| m.path)
            .collect::<Vec<_>>(),
        expected
    );
    assert_eq!(
        run(&ListDirTool::new(root.clone()), ".".into())
            .await
            .unwrap()
            .into_iter()
            .map(|entry| entry.path)
            .collect::<Vec<_>>(),
        vec![PathBuf::from("keep.txt"), "sub".into()]
    );
    assert_eq!(
        run(
            &ReadFileTool::new(root.clone()),
            ReadFileRequest {
                path: "build/output.txt".into(),
                lines: None,
            },
        )
        .await
        .unwrap(),
        "needle\n"
    );
    run(
        &WriteFileTool::new(root.clone()),
        WriteFileRequest {
            path: "build/new.txt".into(),
            content: "written\n".into(),
        },
    )
    .await
    .unwrap();
    run(
        &EditFileTool::new(root),
        EditFileRequest {
            path: "build/new.txt".into(),
            old_string: "written".into(),
            new_string: "edited".into(),
        },
    )
    .await
    .unwrap();
    assert_eq!(
        fs::read_to_string(directory.path().join("build/new.txt")).unwrap(),
        "edited\n"
    );
}

#[tokio::test]
async fn searches_do_not_apply_parent_ignore_files_outside_the_workspace() {
    let directory = tempfile::tempdir().unwrap();
    fs::create_dir(directory.path().join(".git")).unwrap();
    fs::create_dir(directory.path().join("root")).unwrap();
    fs::write(directory.path().join(".gitignore"), "*.rs\n").unwrap();
    fs::write(directory.path().join(".ignore"), "*.txt\n").unwrap();
    for path in ["keep.txt", "keep.rs", "ignored.txt"] {
        fs::write(directory.path().join("root").join(path), "needle\n").unwrap();
    }
    fs::write(directory.path().join("root/.gitignore"), "ignored.txt\n").unwrap();
    let root = WorkspaceRoot::new(directory.path().join("root")).unwrap();
    let expected = vec![PathBuf::from("keep.rs"), "keep.txt".into()];
    assert_eq!(
        run(&ListDirTool::new(root.clone()), ".".into())
            .await
            .unwrap()
            .into_iter()
            .map(|entry| entry.path)
            .collect::<Vec<_>>(),
        expected
    );
    assert_eq!(
        run(&GlobTool::new(root.clone()), glob(".", "**/*"))
            .await
            .unwrap(),
        expected
    );
    assert_eq!(
        run(&GrepTool::new(root), grep(".", "needle"))
            .await
            .unwrap()
            .into_iter()
            .map(|m| m.path)
            .collect::<Vec<_>>(),
        expected
    );
}

#[tokio::test]
async fn searches_preserve_ignore_precedence_and_git_excludes() {
    let directory = tempfile::tempdir().unwrap();
    fs::create_dir_all(directory.path().join(".git/info")).unwrap();
    fs::create_dir(directory.path().join("sub")).unwrap();
    for path in [
        "excluded.txt",
        "gitignored.txt",
        "ignored.txt",
        "keep.txt",
        "sub/keep.txt",
        "sub/ignored.txt",
    ] {
        fs::write(directory.path().join(path), "needle\n").unwrap();
    }
    fs::write(
        directory.path().join(".git/info/exclude"),
        "excluded.txt\nkeep.txt\n",
    )
    .unwrap();
    fs::write(
        directory.path().join(".gitignore"),
        "gitignored.txt\n!keep.txt\n",
    )
    .unwrap();
    fs::write(directory.path().join(".ignore"), "ignored.txt\n").unwrap();
    fs::write(directory.path().join("sub/.gitignore"), "!ignored.txt\n").unwrap();
    let root = WorkspaceRoot::new(directory.path()).unwrap();
    assert_eq!(
        run(&ListDirTool::new(root.clone()), ".".into())
            .await
            .unwrap()
            .into_iter()
            .map(|entry| entry.path)
            .collect::<Vec<_>>(),
        vec![PathBuf::from("keep.txt"), "sub".into()]
    );
    let expected = vec![PathBuf::from("keep.txt"), "sub/keep.txt".into()];
    assert_eq!(
        run(&GlobTool::new(root.clone()), glob(".", "**/*.txt"))
            .await
            .unwrap(),
        expected
    );
    assert_eq!(
        run(&GrepTool::new(root), grep(".", "needle"))
            .await
            .unwrap()
            .into_iter()
            .map(|m| m.path)
            .collect::<Vec<_>>(),
        expected
    );
}

#[tokio::test]
async fn listings_and_searches_ignore_global_git_rules() {
    if let Some(path) = std::env::var_os("FORGE_TOOLS_GLOBAL_IGNORE_ROOT") {
        assert!(
            ignore::gitignore::Gitignore::global()
                .0
                .matched("keep.rs", false)
                .is_ignore()
        );
        let root = WorkspaceRoot::new(path).unwrap();
        let expected = vec![PathBuf::from("keep.rs")];
        assert_eq!(
            run(&ListDirTool::new(root.clone()), ".".into())
                .await
                .unwrap()
                .into_iter()
                .map(|entry| entry.path)
                .collect::<Vec<_>>(),
            expected
        );
        assert_eq!(
            run(&GlobTool::new(root.clone()), glob(".", "**/*"))
                .await
                .unwrap(),
            expected
        );
        assert_eq!(
            run(&GrepTool::new(root), grep(".", "needle"))
                .await
                .unwrap()
                .into_iter()
                .map(|entry| entry.path)
                .collect::<Vec<_>>(),
            expected
        );
        return;
    }

    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("root");
    let excludes = directory.path().join("global-ignore");
    let config = directory.path().join("gitconfig");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("keep.rs"), "needle\n").unwrap();
    fs::write(&excludes, "*.rs\n").unwrap();
    fs::write(
        &config,
        format!("[core]\nexcludesFile = {}\n", excludes.display()),
    )
    .unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "listings_and_searches_ignore_global_git_rules"])
        .env("GIT_CONFIG_GLOBAL", config)
        .env("FORGE_TOOLS_GLOBAL_IGNORE_ROOT", root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
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
            lines_added: 1,
            lines_removed: 0,
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
    use std::{
        os::unix::fs::symlink,
        sync::{
            Arc, Barrier,
            atomic::{AtomicBool, Ordering},
        },
        thread::JoinHandle,
        time::{Duration, Instant},
    };

    fn spawn_dir_swapper(
        workspace: PathBuf,
        outside: PathBuf,
    ) -> (Arc<Barrier>, Arc<AtomicBool>, JoinHandle<usize>) {
        let stop = Arc::new(AtomicBool::new(false));
        let start = Arc::new(Barrier::new(2));
        let thread = {
            let stop = stop.clone();
            let start = start.clone();
            std::thread::spawn(move || {
                let mut flips = 0;
                start.wait();
                while !stop.load(Ordering::Relaxed) {
                    let _ = fs::create_dir(workspace.join("d_real"));
                    let _ = fs::write(workspace.join("d_real/inside.txt"), "inside\n");
                    let _ = fs::rename(workspace.join("d_real"), workspace.join("d"));
                    let _ = symlink(&outside, workspace.join("d_link"));
                    let _ = fs::rename(workspace.join("d"), workspace.join("d_old"));
                    if fs::rename(workspace.join("d_link"), workspace.join("d")).is_ok() {
                        flips += 1;
                    }
                    let _ = fs::remove_dir_all(workspace.join("d_old"));
                    let _ = fs::rename(workspace.join("d"), workspace.join("d_link_old"));
                    let _ = fs::remove_file(workspace.join("d_link_old"));
                }
                flips
            })
        };
        (start, stop, thread)
    }

    #[tokio::test]
    async fn qa10_toctou_dir_swapped_to_external_symlink() {
        use std::sync::{
            Arc, Barrier,
            atomic::{AtomicBool, Ordering},
        };
        use std::time::{Duration, Instant};

        for round in 0..5 {
            let directory = tempfile::tempdir().unwrap();
            let workspace = directory.path().join("root");
            let outside = directory.path().join("outside");
            fs::create_dir(&workspace).unwrap();
            fs::create_dir(&outside).unwrap();
            let tool = WriteFileTool::new(WorkspaceRoot::new(&workspace).unwrap());
            let stop = Arc::new(AtomicBool::new(false));
            let start = Arc::new(Barrier::new(2));
            let flipper = {
                let stop = stop.clone();
                let start = start.clone();
                let outside = outside.clone();
                std::thread::spawn(move || {
                    let mut flips = 0;
                    start.wait();
                    while !stop.load(Ordering::Relaxed) {
                        let _ = fs::create_dir(workspace.join("d_real"));
                        let _ = fs::rename(workspace.join("d_real"), workspace.join("d"));
                        let _ = symlink(&outside, workspace.join("d_link"));
                        let _ = fs::rename(workspace.join("d"), workspace.join("d_old"));
                        if fs::rename(workspace.join("d_link"), workspace.join("d")).is_ok() {
                            flips += 1;
                        }
                        let _ = fs::remove_dir_all(workspace.join("d_old"));
                        let _ = fs::rename(workspace.join("d"), workspace.join("d_link_old"));
                        let _ = fs::remove_file(workspace.join("d_link_old"));
                    }
                    flips
                })
            };
            start.wait();
            let deadline = Instant::now() + Duration::from_secs(1);
            let mut attempts = 0;
            while Instant::now() < deadline && !outside.join("t.txt").exists() {
                attempts += 1;
                let _ = run(
                    &tool,
                    WriteFileRequest {
                        path: "d/t.txt".into(),
                        content: format!("toctou {attempts}\n"),
                    },
                )
                .await;
            }
            stop.store(true, Ordering::Relaxed);
            let flips = flipper.join().unwrap();
            assert!(
                attempts > 0 && flips > 0,
                "no race activity in round {round}"
            );
            assert!(
                !outside.join("t.txt").exists(),
                "write escaped during directory swap in round {round}"
            );
            assert_eq!(fs::read_dir(&outside).unwrap().count(), 0);
        }
    }

    #[tokio::test]
    async fn qa10_toctou_read_never_returns_external_content() {
        for round in 0..5 {
            let directory = tempfile::tempdir().unwrap();
            let workspace = directory.path().join("root");
            let outside = directory.path().join("outside");
            fs::create_dir(&workspace).unwrap();
            fs::create_dir(&outside).unwrap();
            fs::write(outside.join("inside.txt"), "outside-secret\n").unwrap();
            let tool = ReadFileTool::new(WorkspaceRoot::new(&workspace).unwrap());
            let (start, stop, flipper) = spawn_dir_swapper(workspace, outside.clone());
            start.wait();
            let deadline = Instant::now() + Duration::from_secs(1);
            let mut attempts = 0;
            let mut escaped = false;
            while Instant::now() < deadline && !escaped {
                attempts += 1;
                if let Ok(content) = run(
                    &tool,
                    ReadFileRequest {
                        path: "d/inside.txt".into(),
                        lines: None,
                    },
                )
                .await
                {
                    escaped = content != "inside\n";
                }
            }
            stop.store(true, Ordering::Relaxed);
            let flips = flipper.join().unwrap();
            assert!(
                attempts > 0 && flips > 0,
                "no race activity in round {round}"
            );
            assert!(
                !escaped,
                "read escaped during directory swap in round {round}"
            );
            assert_eq!(
                fs::read_to_string(outside.join("inside.txt")).unwrap(),
                "outside-secret\n"
            );
        }
    }

    #[tokio::test]
    async fn qa10_toctou_list_dir_never_lists_external_entries() {
        for round in 0..5 {
            let directory = tempfile::tempdir().unwrap();
            let workspace = directory.path().join("root");
            let outside = directory.path().join("outside");
            fs::create_dir(&workspace).unwrap();
            fs::create_dir(&outside).unwrap();
            fs::write(outside.join("outside-secret.txt"), "outside-secret\n").unwrap();
            let tool = ListDirTool::new(WorkspaceRoot::new(&workspace).unwrap());
            let (start, stop, flipper) = spawn_dir_swapper(workspace, outside.clone());
            start.wait();
            let deadline = Instant::now() + Duration::from_secs(1);
            let mut attempts = 0;
            let mut escaped = false;
            while Instant::now() < deadline && !escaped {
                attempts += 1;
                if let Ok(entries) = run(&tool, "d".into()).await {
                    escaped = entries
                        .iter()
                        .any(|entry| entry.path != PathBuf::from("d/inside.txt"));
                }
            }
            stop.store(true, Ordering::Relaxed);
            let flips = flipper.join().unwrap();
            assert!(
                attempts > 0 && flips > 0,
                "no race activity in round {round}"
            );
            assert!(
                !escaped,
                "list escaped during directory swap in round {round}"
            );
            assert_eq!(fs::read_dir(&outside).unwrap().count(), 1);
        }
    }

    #[tokio::test]
    async fn qa10_toctou_glob_never_matches_external_paths() {
        for round in 0..5 {
            let directory = tempfile::tempdir().unwrap();
            let workspace = directory.path().join("root");
            let outside = directory.path().join("outside");
            fs::create_dir(&workspace).unwrap();
            fs::create_dir(&outside).unwrap();
            fs::write(outside.join("outside-secret.txt"), "outside-secret\n").unwrap();
            let tool = GlobTool::new(WorkspaceRoot::new(&workspace).unwrap());
            let (start, stop, flipper) = spawn_dir_swapper(workspace, outside.clone());
            start.wait();
            let deadline = Instant::now() + Duration::from_secs(1);
            let mut attempts = 0;
            let mut escaped = false;
            while Instant::now() < deadline && !escaped {
                attempts += 1;
                if let Ok(matches) = run(&tool, glob("d", "d/*")).await {
                    escaped = matches
                        .iter()
                        .any(|path| path != &PathBuf::from("d/inside.txt"));
                }
            }
            stop.store(true, Ordering::Relaxed);
            let flips = flipper.join().unwrap();
            assert!(
                attempts > 0 && flips > 0,
                "no race activity in round {round}"
            );
            assert!(
                !escaped,
                "glob escaped during directory swap in round {round}"
            );
            assert_eq!(fs::read_dir(&outside).unwrap().count(), 1);
        }
    }

    #[tokio::test]
    async fn qa10_toctou_grep_never_matches_external_content() {
        for round in 0..5 {
            let directory = tempfile::tempdir().unwrap();
            let workspace = directory.path().join("root");
            let outside = directory.path().join("outside");
            fs::create_dir(&workspace).unwrap();
            fs::create_dir(&outside).unwrap();
            fs::write(outside.join("outside-secret.txt"), "outside-secret\n").unwrap();
            let tool = GrepTool::new(WorkspaceRoot::new(&workspace).unwrap());
            let (start, stop, flipper) = spawn_dir_swapper(workspace, outside.clone());
            start.wait();
            let deadline = Instant::now() + Duration::from_secs(1);
            let mut attempts = 0;
            let mut escaped = false;
            while Instant::now() < deadline && !escaped {
                attempts += 1;
                if let Ok(matches) = run(&tool, grep("d", "outside-secret")).await {
                    escaped = !matches.is_empty();
                }
            }
            stop.store(true, Ordering::Relaxed);
            let flips = flipper.join().unwrap();
            assert!(
                attempts > 0 && flips > 0,
                "no race activity in round {round}"
            );
            assert!(
                !escaped,
                "grep escaped during directory swap in round {round}"
            );
            assert_eq!(fs::read_dir(&outside).unwrap().count(), 1);
        }
    }

    #[tokio::test]
    async fn writes_and_edits_replace_hardlinks_without_modifying_outside_files() {
        let directory = tempfile::tempdir().unwrap();
        fs::create_dir(directory.path().join("root")).unwrap();
        let outside = directory.path().join("outside");
        fs::write(&outside, "original\n").unwrap();
        for name in ["write", "edit"] {
            fs::hard_link(&outside, directory.path().join("root").join(name)).unwrap();
        }
        let root = WorkspaceRoot::new(directory.path().join("root")).unwrap();
        let write = run(
            &WriteFileTool::new(root.clone()),
            WriteFileRequest {
                path: "write".into(),
                content: "written\n".into(),
            },
        )
        .await
        .unwrap();
        assert_eq!(fs::read_to_string(&outside).unwrap(), "original\n");
        let edit = run(
            &EditFileTool::new(root),
            EditFileRequest {
                path: "edit".into(),
                old_string: "original".into(),
                new_string: "edited".into(),
            },
        )
        .await
        .unwrap();
        assert_eq!(fs::read_to_string(&outside).unwrap(), "original\n");
        assert_eq!(
            fs::read_to_string(directory.path().join("root/write")).unwrap(),
            "written\n"
        );
        assert_eq!(
            fs::read_to_string(directory.path().join("root/edit")).unwrap(),
            "edited\n"
        );
        for change in [write, edit] {
            assert_eq!(change.before_sha256, Some(hash("original\n")));
            assert_eq!((change.lines_added, change.lines_removed), (1, 1));
        }
    }

    #[tokio::test]
    async fn all_tools_keep_using_the_opened_workspace_after_its_path_is_replaced() {
        let directory = tempfile::tempdir().unwrap();
        fs::create_dir(directory.path().join("root")).unwrap();
        fs::create_dir(directory.path().join("outside")).unwrap();
        fs::write(directory.path().join("root/a"), "inside\n").unwrap();
        fs::write(directory.path().join("outside/a"), "outside\n").unwrap();
        let root = WorkspaceRoot::new(directory.path().join("root")).unwrap();
        fs::rename(
            directory.path().join("root"),
            directory.path().join("opened"),
        )
        .unwrap();
        symlink("outside", directory.path().join("root")).unwrap();
        assert_eq!(
            run(
                &ReadFileTool::new(root.clone()),
                ReadFileRequest {
                    path: "a".into(),
                    lines: None
                }
            )
            .await
            .unwrap(),
            "inside\n"
        );
        let write = run(
            &WriteFileTool::new(root.clone()),
            WriteFileRequest {
                path: "b".into(),
                content: "inside\n".into(),
            },
        )
        .await
        .unwrap();
        assert_eq!(write.path, PathBuf::from("b"));
        let edit = run(
            &EditFileTool::new(root.clone()),
            EditFileRequest {
                path: "a".into(),
                old_string: "inside".into(),
                new_string: "inside edited".into(),
            },
        )
        .await
        .unwrap();
        assert_eq!(edit.path, PathBuf::from("a"));
        let expected = vec![PathBuf::from("a"), "b".into()];
        assert_eq!(
            run(&ListDirTool::new(root.clone()), ".".into())
                .await
                .unwrap()
                .into_iter()
                .map(|e| e.path)
                .collect::<Vec<_>>(),
            expected
        );
        assert_eq!(
            run(&GlobTool::new(root.clone()), glob(".", "**/*"))
                .await
                .unwrap(),
            expected
        );
        assert_eq!(
            run(&GrepTool::new(root), grep(".", "inside"))
                .await
                .unwrap()
                .into_iter()
                .map(|m| m.path)
                .collect::<Vec<_>>(),
            expected
        );
        assert_eq!(
            fs::read_to_string(directory.path().join("outside/a")).unwrap(),
            "outside\n"
        );
        assert!(!directory.path().join("outside/b").exists());
    }

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

    #[tokio::test]
    async fn write_file_rejects_absolute_external_symlink_targets() {
        let directory = tempfile::tempdir().unwrap();
        fs::create_dir(directory.path().join("root")).unwrap();
        fs::create_dir(directory.path().join("outside")).unwrap();
        fs::write(directory.path().join("outside/file"), "original").unwrap();
        symlink(
            directory.path().join("outside/file"),
            directory.path().join("root/link"),
        )
        .unwrap();
        symlink(
            directory.path().join("outside"),
            directory.path().join("root/parent"),
        )
        .unwrap();
        let tool = WriteFileTool::new(WorkspaceRoot::new(directory.path().join("root")).unwrap());
        for path in ["link", "parent", "parent/file", "parent/new"] {
            assert!(
                run(
                    &tool,
                    WriteFileRequest {
                        path: path.into(),
                        content: "changed".into(),
                    }
                )
                .await
                .is_err(),
                "{path}"
            );
        }
        assert_eq!(
            fs::read_to_string(directory.path().join("outside/file")).unwrap(),
            "original"
        );
        assert_eq!(
            fs::read_dir(directory.path().join("outside"))
                .unwrap()
                .count(),
            1
        );
        for name in ["link", "parent"] {
            assert!(
                fs::symlink_metadata(directory.path().join("root").join(name))
                    .unwrap()
                    .is_symlink()
            );
        }
        assert_eq!(
            entry_names(&directory.path().join("root")),
            ["link", "parent"]
        );
    }

    #[tokio::test]
    async fn write_file_through_in_root_final_symlink_replaces_target_not_link() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(directory.path().join("real"), "original").unwrap();
        symlink("real", directory.path().join("link")).unwrap();
        let root = WorkspaceRoot::new(directory.path()).unwrap();
        let change = run(
            &WriteFileTool::new(root),
            WriteFileRequest {
                path: "link".into(),
                content: "changed".into(),
            },
        )
        .await
        .unwrap();
        assert_eq!(change.path, PathBuf::from("real"));
        assert_eq!(
            fs::read_to_string(directory.path().join("real")).unwrap(),
            "changed"
        );
        assert_eq!(
            fs::read_link(directory.path().join("link")).unwrap(),
            PathBuf::from("real")
        );
        assert_eq!(entry_names(directory.path()), ["link", "real"]);
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
