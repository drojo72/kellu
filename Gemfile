# frozen_string_literal: true

source "https://rubygems.org"

# CLI robusta (Thor facilita comandos con subcomandos y opciones)
gem "thor", "~> 1.3"

# Base de datos SQLite (para FTS5 y sqlite-vec)
gem "sqlite3", "~> 1.7"

# Cliente HTTP eficiente para APIs de IA y OpenRouter
gem "http", "~> 5.2"

# Manejo seguro de secretos mediante el Keyring del sistema (libsecret)
gem "keyring", "~> 0.4.1"

# Parseo de archivos de configuración TOML
gem "toml-rb", "~> 2.2"

group :development, :test do
  gem "minitest", "~> 5.20"
  gem "rubocop", "~> 1.60", require: false
end
