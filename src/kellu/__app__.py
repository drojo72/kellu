"""Single source of truth for the application's identity.

Everything that depends on the project name (paths, environment prefix, keyring schema)
derives from here, so a rename touches one file.
"""

__version__ = "0.1.0.dev0"

APP_NAME = "kellu"
APP_DISPLAY_NAME = "Kellu"
ENV_PREFIX = "KELLU_"

# Keyring entries carry one attribute named like the app, whose value is "<service>_<key>"
# (for example ``kellu=ai_openrouter_api_key``), plus the label "KELLU: <name>".
KEYRING_ATTRIBUTE = "kellu"
# Attribute used by the earlier prototype; read only by the migration command (M3).
LEGACY_KEYRING_ATTRIBUTE = "kai"
