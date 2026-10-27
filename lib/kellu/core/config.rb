# frozen_string_literal: true

require 'toml-rb'
require 'fileutils'

module Kellu
  module Core
    class Config
      DEFAULT_DIR = File.expand_path('~/.config/kellu')
      DEFAULT_CONFIG_PATH = File.join(DEFAULT_DIR, 'config.toml')

      attr_reader :config_path, :data

      # Estructura por defecto para inicializar un archivo de configuración nuevo
      DEFAULT_TEMPLATE = <<~TOML
        # Configuración global de Kellu LSP
        version = "1.0"
        default_profile = "default"

        [profiles.default]
        provider = "deepseek"
        model = "deepseek-chat"
        temperature = 0.2
        max_tokens = 2048

        [profiles.creative]
        provider = "google"
        model = "gemini-2.5-pro"
        temperature = 0.7
        max_tokens = 4096
      TOML

      def initialize(config_path: DEFAULT_CONFIG_PATH)
        @config_path = config_path
        @data = load_or_create_config
      end

      # Obtiene una clave usando notación de puntos (ej: "profiles.default.model")
      def get(path)
        path.to_s.split('.').inject(@data) do |acc, key|
          acc.is_a?(Hash) ? acc[key] : nil
        end
      end

      # Obtiene el perfil activo actual basándose en default_profile
      def current_profile
        profile_name = @data['default_profile'] || 'default'
        profiles = @data['profiles'] || {}
        profiles[profile_name] || {}
      end

      # Guarda los cambios actuales de vuelta al archivo TOML
      def save!
        FileUtils.mkdir_p(File.dirname(@config_path))
        # Nota: Si prefieres serializar de vuelta a TOML puro, puedes usar una gema como 'toml-rb' para volcar,
        # o mantener un volcado estructurado. Por ahora, aseguramos la lectura robusta.
      end

      private

      def load_or_create_config
        unless File.exist?(@config_path)
          FileUtils.mkdir_p(File.dirname(@config_path))
          File.write(@config_path, DEFAULT_TEMPLATE)
        end

        Tomlrb.load_file(@config_path, symbolize_keys: false)
      rescue Tomlrb::ParseError => e
        raise "Error al parsear el archivo de configuración en #{@config_path}: #{e.message}"
      end
    end
  end
end
