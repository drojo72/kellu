# frozen_string_literal: true

require 'toml-rb'
require 'fileutils'

module Kellu
  module Core
    class Config
      DEFAULT_DIR = File.expand_path('~/.config/kellu')
      DEFAULT_PATH = File.join(DEFAULT_DIR, 'config.toml')

      DEFAULT_SETTINGS = {
        'general' => {
          'default_provider' => 'deepseek',
          'default_model' => 'deepseek-chat',
          'timeout' => 30
        },
        'providers' => {
          'deepseek' => {
            'enabled' => true,
            'api_base' => 'https://api.deepseek.com/v1',
            'model' => 'deepseek-chat'
          },
          'openrouter' => {
            'enabled' => true,
            'api_base' => 'https://openrouter.ai/api/v1',
            'model' => 'anthropic/claude-3.5-sonnet'
          },
          'google' => {
            'enabled' => true,
            'api_base' => 'https://generativelanguage.googleapis.com/v1beta',
            'model' => 'gemini-2.5-pro'
          },
          'gemma' => {
            'enabled' => true,
            'api_base' => 'https://generativelanguage.googleapis.com/v1beta',
            'model' => 'gemma4'
          },
        }
      }.freeze

      attr_reader :config_path, :data

      def initialize(config_path = DEFAULT_PATH)
        @config_path = config_path
        @data = load_or_initialize
      end

      # Permite acceder a las secciones como métodos o claves
      def [](section)
        @data[section.to_s]
      end

      def general
        @data['general'] || {}
      end

      def provider_config(name)
        providers = @data['providers'] || {}
        providers[name.to_s] || {}
      end

      def save!
        FileUtils.mkdir_p(File.dirname(@config_path))
        File.write(@config_path, serialize_toml(@data))
      end

      private

      def load_or_initialize
        unless File.exist?(@config_path)
          FileUtils.mkdir_p(File.dirname(@config_path))
          @data = DEFAULT_SETTINGS.dup
          save!
          return @data
        end

        begin
          TomlRB.load_file(@config_path, symbolize_keys: false)
        rescue StandardError => e
          raise "Error al parsear el archivo de configuración en #{@config_path}: #{e.message}"
        end
      end

      # Serializador simple a formato TOML para evitar gemas pesadas de escritura
      def serialize_toml(hash, prefix = '')
        lines = []
        tables = []

        hash.each do |key, value|
          if value.is_a?(Hash)
            tables << [prefix.empty? ? key : "#{prefix}.#{key}", value]
          else
            lines << "#{key} = #{format_toml_value(value)}"
          end
        end

        tables.each do |table_name, sub_hash|
          lines << ""
          lines << "[#{table_name}]"
          sub_hash.each do |sub_key, sub_val|
            if sub_val.is_a?(Hash)
              # Manejo de sub-tablas anidadas si fuera necesario
              tables << ["#{table_name}.#{sub_key}", sub_val]
            else
              lines << "#{sub_key} = #{format_toml_value(sub_val)}"
            end
          end
        end

        lines.join("\n") + "\n"
      end

      def format_toml_value(value)
        case value
        when String
          "\"#{value}\""
        when TrueClass, FalseClass
          value.to_s
        when Numeric
          value.to_s
        else
          "\"#{value}\""
        end
      end
    end
  end
end
