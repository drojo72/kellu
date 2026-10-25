from __future__ import annotations

from pathlib import Path

import pytest

from kellu import __version__
from kellu.cli import EXIT_ERROR, EXIT_NOT_IMPLEMENTED, EXIT_OK, main


def test_version_flag(capsys: pytest.CaptureFixture[str]) -> None:
    with pytest.raises(SystemExit) as exc:
        main(["--version"])
    assert exc.value.code == 0
    assert __version__ in capsys.readouterr().out


def test_no_command_prints_help(capsys: pytest.CaptureFixture[str]) -> None:
    assert main([]) == EXIT_OK
    assert "usage:" in capsys.readouterr().out


@pytest.mark.parametrize("command", ["keys", "tools", "lsp"])
def test_planned_commands_report_not_implemented(
    command: str, capsys: pytest.CaptureFixture[str]
) -> None:
    assert main([command]) == EXIT_NOT_IMPLEMENTED
    assert "not implemented yet" in capsys.readouterr().err


def test_unknown_command_is_a_usage_error() -> None:
    with pytest.raises(SystemExit) as exc:
        main(["nope"])
    assert exc.value.code == 2


def test_config_requires_an_action() -> None:
    with pytest.raises(SystemExit) as exc:
        main(["config"])
    assert exc.value.code == 2


def test_config_path_prints_the_user_file_location(
    fake_home: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    assert main(["config", "path"]) == EXIT_OK
    captured = capsys.readouterr()
    assert captured.out.strip() == str(fake_home / ".config" / "kellu" / "config.toml")
    assert "does not exist" in captured.err


def test_config_check_with_defaults_succeeds(
    fake_home: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    assert main(["config", "check"]) == EXIT_OK
    assert "ok:" in capsys.readouterr().out


def test_config_check_reports_a_rejected_secret(
    fake_home: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    directory = fake_home / ".config" / "kellu"
    directory.mkdir(parents=True)
    user = directory / "config.toml"
    user.write_text('[providers.deepseek]\napi_key = "sk-nope"\n', encoding="utf-8")
    user.chmod(0o600)
    assert main(["config", "check"]) == EXIT_ERROR
    assert "keyring" in capsys.readouterr().err
