from __future__ import annotations

import os
from pathlib import Path
from typing import Any

import pytest

from kellu.common.errors import ConfigError, SecurityError
from kellu.core import config as cfg


def _write(path: Path, text: str, mode: int = 0o600) -> Path:
    path.write_text(text, encoding="utf-8")
    path.chmod(mode)
    return path


def _parse(user: dict[str, Any]) -> cfg.Config:
    return cfg.parse_config(cfg.load_defaults(), user)


def test_packaged_defaults_are_valid() -> None:
    config = cfg.parse_config(cfg.load_defaults())
    assert set(config.providers) == {"openrouter", "deepseek", "google", "gemma_direct", "grok"}
    assert config.providers["google"].model == "gemini-flash-latest"
    assert config.providers["gemma_direct"].model.startswith("gemma")
    assert config.providers["gemma_direct"].keyring_name == "gemma_direct"
    assert config.route("code").chain == ("deepseek", "openrouter")
    assert config.route("general").chain[0] == "gemma_direct"
    assert config.router.total_timeout_s == 120.0
    assert config.logging.level == "INFO"
    assert config.sessions.enabled is True


def test_unknown_task_falls_back_to_general_route() -> None:
    config = cfg.parse_config(cfg.load_defaults())
    assert config.route("something-else") == config.routing["general"]


def test_missing_user_file_means_defaults_only(fake_home: Path) -> None:
    config = cfg.load_config()
    assert config.source is None
    assert not (fake_home / ".config").exists()  # loading creates nothing


def test_user_file_overrides_part_of_a_provider(tmp_path: Path) -> None:
    user = _write(tmp_path / "config.toml", '[providers.deepseek]\nmodel = "deepseek-reasoner"\n')
    config = cfg.load_config(user)
    assert config.providers["deepseek"].model == "deepseek-reasoner"
    assert config.providers["deepseek"].base_url == "https://api.deepseek.com/v1"
    assert config.source == user


def test_user_can_add_provider_and_route(tmp_path: Path) -> None:
    user = _write(
        tmp_path / "config.toml",
        '[providers.local]\nkind = "openai_compat"\nbase_url = "http://localhost:11434/v1"\n'
        'model = "qwen"\ntimeout_s = 120\n\n'
        '[routing.code]\nprimary = "local"\nbackup = ["deepseek"]\n',
    )
    config = cfg.load_config(user)
    assert config.providers["local"].keyring_name is None
    assert config.route("code").chain == ("local", "deepseek")


@pytest.mark.parametrize(
    "key", ["api_key", "token", "password", "openrouter_api_key", "db_password", "API_KEY"]
)
def test_secret_looking_keys_are_rejected(key: str) -> None:
    with pytest.raises(ConfigError, match="keyring"):
        _parse({"providers": {"openrouter": {key: "sk-should-not-be-here"}}})


def test_legitimate_keys_with_similar_words_are_not_flagged() -> None:
    config = _parse({"sessions": {"max_entry_chars": 500}})
    assert config.sessions.max_entry_chars == 500


def test_unknown_top_level_key_is_rejected() -> None:
    with pytest.raises(ConfigError, match="unknown key"):
        _parse({"provders": {}})


def test_unknown_provider_field_is_rejected() -> None:
    with pytest.raises(ConfigError, match="unknown key"):
        _parse({"providers": {"deepseek": {"modle": "x"}}})


def test_plain_http_is_only_allowed_for_localhost() -> None:
    with pytest.raises(ConfigError, match="plain http"):
        _parse({"providers": {"deepseek": {"base_url": "http://api.example.com/v1"}}})
    config = _parse({"providers": {"deepseek": {"base_url": "http://127.0.0.1:8080/v1/"}}})
    assert config.providers["deepseek"].base_url == "http://127.0.0.1:8080/v1"


def test_credentials_in_urls_are_rejected() -> None:
    with pytest.raises(ConfigError, match="credentials"):
        _parse({"providers": {"deepseek": {"base_url": "https://user:pw@api.deepseek.com/v1"}}})


def test_non_http_schemes_are_rejected() -> None:
    with pytest.raises(ConfigError, match="http"):
        _parse({"providers": {"deepseek": {"base_url": "file:///etc/passwd"}}})


def test_route_must_reference_known_providers() -> None:
    with pytest.raises(ConfigError, match="unknown provider"):
        _parse({"routing": {"code": {"primary": "nope"}}})


def test_route_rejects_duplicate_providers() -> None:
    with pytest.raises(ConfigError, match="more than once"):
        _parse({"routing": {"code": {"primary": "deepseek", "backup": ["deepseek"]}}})


@pytest.mark.parametrize("timeout", [0, 301, "60", True])
def test_timeout_must_be_a_number_in_range(timeout: object) -> None:
    with pytest.raises(ConfigError, match="timeout_s"):
        _parse({"providers": {"deepseek": {"timeout_s": timeout}}})


def test_log_level_is_validated_and_normalized() -> None:
    assert _parse({"logging": {"level": "debug"}}).logging.level == "DEBUG"
    with pytest.raises(ConfigError, match="logging.level"):
        _parse({"logging": {"level": "chatty"}})


def test_invalid_toml_is_a_config_error(tmp_path: Path) -> None:
    user = _write(tmp_path / "config.toml", "this is = = not toml")
    with pytest.raises(ConfigError, match="invalid TOML"):
        cfg.load_config(user)


def test_oversized_file_is_rejected(tmp_path: Path) -> None:
    user = _write(tmp_path / "config.toml", "# " + "x" * cfg.MAX_CONFIG_BYTES)
    with pytest.raises(ConfigError, match="larger than"):
        cfg.load_config(user)


def test_group_or_world_writable_file_is_rejected(tmp_path: Path) -> None:
    user = _write(tmp_path / "config.toml", "[logging]\nlevel = 'INFO'\n", mode=0o666)
    with pytest.raises(SecurityError, match="writable by group or others"):
        cfg.load_config(user)


def test_symlinked_config_is_rejected(tmp_path: Path) -> None:
    real = _write(tmp_path / "real.toml", "[logging]\nlevel = 'INFO'\n")
    link = tmp_path / "config.toml"
    os.symlink(real, link)
    with pytest.raises(SecurityError, match="symlink"):
        cfg.load_config(link)


def test_config_is_immutable() -> None:
    config = cfg.parse_config(cfg.load_defaults())
    with pytest.raises(TypeError):
        config.providers["x"] = config.providers["google"]  # type: ignore[index]


@pytest.mark.parametrize("value", [0, 4, 901, "60", True])
def test_router_total_timeout_is_validated(value: object) -> None:
    with pytest.raises(ConfigError, match="total_timeout_s"):
        _parse({"router": {"total_timeout_s": value}})


def test_router_unknown_key_is_rejected() -> None:
    with pytest.raises(ConfigError, match="unknown key"):
        _parse({"router": {"retries": 3}})


def test_provider_kinds_match_the_registry() -> None:
    from kellu.providers.registry import KINDS

    assert cfg.PROVIDER_KINDS == KINDS
