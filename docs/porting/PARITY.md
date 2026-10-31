# Parity register

Each function of `python-tests.txt` needs a Rust counterpart or a written reason. Gate for
leaving M2: no `pending` row. Only M0 files are mapped so far; the rest are listed by file.

## M0

| Python test | Rust | Status |
|---|---|---|
| test_architecture.test_every_source_directory_is_a_declared_layer | `kellu/tests/architecture.rs::every_workspace_crate_is_a_declared_layer` | done |
| test_architecture.test_layer_only_imports_allowed_layers | `architecture.rs::crates_only_depend_on_allowed_layers` | done |
| test_version.test_version_is_single_sourced | `architecture.rs::the_version_is_single_sourced`, `app::tests::identity_is_consistent` | done |
| test_paths.test_absolute_xdg_variable_is_honored | `paths::tests::absolute_xdg_variable_is_honored` | done |
| test_paths.test_defaults_follow_xdg_layout | `paths::tests::defaults_follow_xdg_layout` | done |
| test_paths.test_relative_xdg_variable_is_ignored | `paths::tests::relative_or_empty_xdg_variable_is_ignored` | done |
| test_paths.test_resolving_paths_does_not_create_anything | `paths::tests::resolving_paths_does_not_create_anything` | done |
| test_paths.test_ensure_private_dir_creates_mode_0700 | same name | done |
| test_paths.test_ensure_private_dir_rejects_symlink | same name | done |
| test_paths.test_ensure_private_dir_tightens_loose_permissions | same name | done |
| test_paths.test_ensure_private_file_creates_empty_private_file | same name | done |
| test_paths.test_open_private_append_appends | same name | done |
| test_paths.test_open_private_append_creates_mode_0600 | same name | done |
| test_paths.test_open_private_append_new_only_rejects_existing | same name | done |
| test_paths.test_open_private_append_refuses_symlink | same name | done |
| test_paths.test_open_private_append_tightens_existing_permissions | same name | done |
| test_cli.test_version_flag | `kellu/tests/cli.rs::version_flag` | done |
| test_cli.test_no_command_prints_help | `cli.rs::no_command_prints_help` | done |
| test_cli.test_unknown_command_is_a_usage_error | `cli.rs::unknown_command_is_a_usage_error` | done |
| test_cli.test_planned_commands_report_not_implemented | `cli.rs::planned_commands_report_not_implemented` | done |
| test_doctor.test_all_tools_present_passes | `doctor::tests::all_tools_present_passes` | done |
| test_doctor.test_checks_are_marked_required_or_optional | same name | done |
| test_doctor.test_missing_optional_tool_only_warns | same name | done |
| test_doctor.test_missing_required_tool_fails | same name | done |
| test_doctor.test_invalid_config_fails_the_check | needs config | pending (M1) |

Not applicable: the Python version check in `doctor` (no interpreter in Rust).

Extra negative tests added: dangling symlink never creates its target, non-regular files refused,
foreign-owner directory refused (runs when tests execute as root), non-executable file is not a
tool, `doctor` creates nothing, missing or relative `HOME` is an error and not a panic.

## Pending

| Python file | Functions | Milestone |
|---|---|---|
| test_config.py, test_log.py, test_session.py, test_text.py, test_cli.py (`config` tests) | 24 + 8 + 11 + 7 + 5 | M1 |
| test_providers_openai.py, test_providers_google.py, test_router.py | 26 + 12 + 19 | M2 |
