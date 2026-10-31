<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="/assets/branding/svg/kellu-logo-color-dark.svg">
    <img alt="Kellu" src="/assets/branding/svg/kellu-logo-color-light.svg" width="350">
  </picture>
</p>
<p align="center">
    <em>Local-first, modular AI assistant & LSP CLI built with Ruby & Rust</em>
</p>

---

# Parity register

Each function of `docs/porting/python-tests.txt` (137 functions) needs a Rust counterpart or a
written reason it does not apply. Gate for leaving M2: no `pending` row.

Status at the close of M1: **80 of 137 functions mapped** (M0 and M1), 57 pending (M2).
Names without a module prefix are in `kellu/tests/cli.rs` or `kellu/tests/architecture.rs`
(end-to-end tests of the real binary and of `cargo metadata`).

## M0

| Python test | Rust | Status |
|---|---|---|
| test_architecture.test_every_source_directory_is_a_declared_layer | `every_workspace_crate_is_a_declared_layer` | done |
| test_architecture.test_layer_only_imports_allowed_layers | `crates_only_depend_on_allowed_layers` | done |
| test_version.test_version_is_single_sourced | `the_version_is_single_sourced`, `app::tests::identity_is_consistent` | done |
| test_paths.test_absolute_xdg_variable_is_honored | `paths::tests::absolute_xdg_variable_is_honored` | done |
| test_paths.test_defaults_follow_xdg_layout | `paths::tests::defaults_follow_xdg_layout` | done |
| test_paths.test_relative_xdg_variable_is_ignored | `paths::tests::relative_or_empty_xdg_variable_is_ignored` | done |
| test_paths.test_resolving_paths_does_not_create_anything | `paths::tests::resolving_paths_does_not_create_anything` | done |
| test_paths.test_ensure_private_dir_creates_mode_0700 | `paths::tests::ensure_private_dir_creates_mode_0700` | done |
| test_paths.test_ensure_private_dir_rejects_symlink | `paths::tests::ensure_private_dir_rejects_symlink` | done |
| test_paths.test_ensure_private_dir_tightens_loose_permissions | `paths::tests::ensure_private_dir_tightens_loose_permissions` | done |
| test_paths.test_ensure_private_file_creates_empty_private_file | `paths::tests::ensure_private_file_creates_empty_private_file` | done |
| test_paths.test_open_private_append_appends | `paths::tests::open_private_append_appends` | done |
| test_paths.test_open_private_append_creates_mode_0600 | `paths::tests::open_private_append_creates_mode_0600` | done |
| test_paths.test_open_private_append_new_only_rejects_existing | `paths::tests::open_private_append_new_only_rejects_existing` | done |
| test_paths.test_open_private_append_refuses_symlink | `paths::tests::open_private_append_refuses_symlink` | done |
| test_paths.test_open_private_append_tightens_existing_permissions | `paths::tests::open_private_append_tightens_existing_permissions` | done |
| test_cli.test_version_flag | `version_flag` | done |
| test_cli.test_no_command_prints_help | `no_command_prints_help` | done |
| test_cli.test_unknown_command_is_a_usage_error | `unknown_command_is_a_usage_error` | done |
| test_cli.test_planned_commands_report_not_implemented | `planned_commands_report_not_implemented` | done |
| test_doctor.test_all_tools_present_passes | `doctor::tests::all_tools_present_passes` | done |
| test_doctor.test_checks_are_marked_required_or_optional | `doctor::tests::checks_are_marked_required_or_optional` | done |
| test_doctor.test_missing_optional_tool_only_warns | `doctor::tests::missing_optional_tool_only_warns` | done |
| test_doctor.test_missing_required_tool_fails | `doctor::tests::missing_required_tool_fails` | done |

## M1

| Python test | Rust | Status |
|---|---|---|
| test_doctor.test_invalid_config_fails_the_check | `doctor::tests::invalid_config_fails_the_check`, `doctor_fails_on_an_invalid_config` | done |
| test_cli.test_config_check_reports_a_rejected_secret | `config_check_reports_a_rejected_secret_without_echoing_it` | done |
| test_cli.test_config_check_with_defaults_succeeds | `config_check_with_defaults_succeeds_and_creates_nothing` | done |
| test_cli.test_config_path_prints_the_user_file_location | `config_path_prints_the_user_file_location` | done |
| test_cli.test_config_requires_an_action | `config_requires_an_action` | done |
| test_cli.test_config_show_lists_providers_and_routes_without_secrets | `config_show_lists_providers_and_routes_without_secrets` | done |
| test_config.test_config_is_immutable | Not a test: `Config` and its parts expose only `&` accessors and have no `&mut` API, so immutability is enforced by the type system | n/a (reason) |
| test_config.test_credentials_in_urls_are_rejected | `config::tests::credentials_in_urls_are_rejected`, `net::tests::credentials_in_urls_are_rejected` | done |
| test_config.test_group_or_world_writable_file_is_rejected | `config::tests::group_or_world_writable_file_is_rejected` | done |
| test_config.test_invalid_toml_is_a_config_error | `config::tests::invalid_toml_is_a_config_error_with_a_line_and_no_content` | done |
| test_config.test_legitimate_keys_with_similar_words_are_not_flagged | `config::tests::legitimate_keys_with_similar_words_are_not_flagged` | done |
| test_config.test_log_level_is_validated_and_normalized | `config::tests::log_level_is_validated_and_normalized` | done |
| test_config.test_missing_user_file_means_defaults_only | `config::tests::missing_user_file_means_defaults_only` | done |
| test_config.test_non_http_schemes_are_rejected | `config::tests::non_http_schemes_are_rejected` | done |
| test_config.test_oversized_file_is_rejected | `config::tests::oversized_file_is_rejected`, `a_file_exactly_at_the_limit_is_accepted` | done |
| test_config.test_packaged_defaults_are_valid | `config::tests::packaged_defaults_are_valid`, `default_routes_match_the_design` | done |
| test_config.test_plain_http_is_only_allowed_for_localhost | `config::tests::plain_http_is_only_allowed_for_localhost` | done |
| test_config.test_provider_kinds_match_the_registry | `config::tests::provider_kinds_match_the_registry` (M1: the kind list lives in `kellu-providers::types`; the registry itself arrives in M2, which must keep this test) | done (revisit in M2) |
| test_config.test_route_must_reference_known_providers | `config::tests::route_must_reference_known_providers` | done |
| test_config.test_route_rejects_duplicate_providers | `config::tests::route_rejects_duplicate_providers` | done |
| test_config.test_router_total_timeout_is_validated | `config::tests::router_total_timeout_is_validated` | done |
| test_config.test_router_unknown_key_is_rejected | `config::tests::router_unknown_key_is_rejected` | done |
| test_config.test_secret_looking_keys_are_rejected | `config::tests::secret_looking_keys_are_rejected_with_a_pointer_to_the_keyring`, `a_secret_in_the_user_file_is_rejected_when_loading` | done |
| test_config.test_symlinked_config_is_rejected | `config::tests::symlinked_config_is_rejected` | done |
| test_config.test_timeout_must_be_a_number_in_range | `config::tests::timeout_must_be_a_number_in_range`, `numeric_limits_are_enforced` | done |
| test_config.test_unknown_provider_field_is_rejected | `config::tests::unknown_provider_field_is_rejected` | done |
| test_config.test_unknown_task_falls_back_to_general_route | `config::tests::unknown_task_falls_back_to_general_route` | done |
| test_config.test_unknown_top_level_key_is_rejected | `config::tests::unknown_top_level_key_is_rejected` | done |
| test_config.test_user_can_add_provider_and_route | `config::tests::user_can_add_provider_and_route` | done |
| test_config.test_user_file_overrides_part_of_a_provider | `config::tests::user_file_overrides_part_of_a_provider` | done |
| test_log.test_configuring_twice_does_not_duplicate_output | `configuring_twice_replaces_the_sink_and_does_not_duplicate_output` (`kellu-core/tests/logging.rs`, its own process because the `log` facade is global) | done |
| test_log.test_control_characters_are_stripped | `log::tests::control_characters_are_stripped` | done |
| test_log.test_creates_private_directory_and_file | `log::tests::creates_private_directory_and_file` | done |
| test_log.test_importing_the_module_creates_nothing | Rust has no import-time execution. Construction side effects are covered by `config::tests::loading_creates_nothing_on_disk` and `session::tests::building_a_session_touches_nothing` | n/a (reason) |
| test_log.test_logger_does_not_propagate_to_root | `log::tests::records_from_other_crates_are_not_written` (the `log` facade has one global logger; our sink only accepts records from Kellu targets) | done (adapted) |
| test_log.test_redaction_applies_to_messages_and_exceptions | `log::tests::redaction_applies_to_messages_and_error_text` | done |
| test_log.test_rotated_files_stay_private | `log::tests::rotation_keeps_the_newest_backups_and_stays_private` | done |
| test_log.test_unknown_level_is_rejected | `log::tests::unknown_levels_are_rejected` | done |
| test_session.test_content_cannot_forge_headings_or_close_the_fence | `session::tests::content_cannot_forge_headings_or_close_the_fence`, `fences_grow_with_the_longest_backtick_run` | done |
| test_session.test_context_manager_ends_the_session | `session::tests::dropping_an_active_session_leaves_a_complete_file` (`Drop` replaces the context manager) | done (adapted) |
| test_session.test_empty_label_uses_default_slug | `session::tests::empty_label_uses_default_slug` | done |
| test_session.test_end_appends_next_steps_and_closes | `session::tests::end_appends_next_steps_and_closes` | done |
| test_session.test_entries_are_redacted_cleaned_and_truncated | `session::tests::entries_are_redacted_cleaned_and_truncated` | done |
| test_session.test_hostile_label_cannot_escape_the_directory | `session::tests::hostile_label_cannot_escape_the_directory` | done |
| test_session.test_logging_before_start_or_after_end_fails | `session::tests::logging_before_start_or_after_end_fails` | done |
| test_session.test_provider_name_is_sanitized | `session::tests::provider_name_is_sanitized` | done |
| test_session.test_same_second_sessions_get_unique_files | `session::tests::same_second_sessions_get_unique_files` | done |
| test_session.test_start_creates_private_file_with_header | `session::tests::start_creates_private_file_with_header` | done |
| test_session.test_unknown_kind_is_rejected | `session::tests::unknown_kind_is_rejected` (entry kinds are an enum, so the check is `EntryKind::parse`) | done (adapted) |
| test_text.test_identity_redactor_returns_text_unchanged | `text::tests::identity_redactor_returns_text_unchanged` | done |
| test_text.test_strip_control_chars_keeps_newline_and_tab | `text::tests::strip_control_chars_keeps_newline_and_tab` | done |
| test_text.test_strip_control_chars_removes_bidi_override | `text::tests::strip_control_chars_removes_bidi_override_and_zero_width` and `..._covers_the_format_character_blocks` | done |
| test_text.test_strip_control_chars_removes_escape_sequences_and_carriage_returns | `text::tests::strip_control_chars_removes_escape_sequences_and_carriage_returns` | done |
| test_text.test_truncate_leaves_short_text_alone | `text::tests::truncate_leaves_short_text_alone` | done |
| test_text.test_truncate_rejects_non_positive_limit | The limit is a `NonZeroUsize`, so a zero limit does not compile | n/a (reason) |
| test_text.test_truncate_reports_dropped_characters | `text::tests::truncate_reports_dropped_characters`, `truncate_counts_characters_not_bytes` | done |

## Behavior differences from the Python prototype (decided, not accidental)

| Area | Python | Rust | Why |
|---|---|---|---|
| Timestamps in log and session files | local time (`datetime.now`) | **UTC** (`kellu-common::utc`) | Local time needs the platform time-zone database, which is not safe to read from a multithreaded program, and the `time` crate would add a dependency for it. Session file names and headers therefore show UTC. Revisit if local time matters to users. |
| Configuration parsing | dict plus hand checks | `toml` read as a table, validated by hand | Errors name the key path and never echo a value, which `serde` derive errors can do. |
| URL parsing | hand-written | `http::Uri` | One parser for validation and (from M2) transport, no WHATWG differential, no ICU dependency tree. |

## Extra negative tests added in the port

Dangling symlink never creates its target; non-regular files refused; foreign-owner directory and
configuration file refused (these two run when the tests execute as root); a FIFO or directory in
place of the configuration file is rejected without hanging; configuration file exactly at the
size limit accepted; non-UTF-8 configuration is an error; configuration errors never echo values
and bound and sanitize keys taken from the file; a message cannot forge another log line; a
symlinked log file or state directory is refused and its target left untouched; a secret cut by
truncation is never exposed; a secret in the session label is redacted before the file name is
made; an existing dangling symlink is skipped, not followed, when allocating a session file;
`doctor` creates nothing; missing or relative `HOME` is an error and not a panic.

## Pending (M2)

| Python file | Functions | Milestone |
|---|---|---|
| test_providers_openai.py | 26 | M2 |
| test_providers_google.py | 12 | M2 |
| test_router.py | 19 | M2 |
