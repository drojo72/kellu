# frozen_string_literal: true

require_relative 'config'
require_relative '../security/keyring'

module Kellu
  module Core
    class Router
      attr_reader :config, :keyring

      def initialize(config: Config.new, keyring: Security::KeyringManager.new)
        @config = config
        @keyring = keyring
      end

      # Obtiene los detalles del proveedor activo y su clave de API correspondiente
      def active_target
        profile = @config.current_profile
        provider = profile['provider'] || 'deepseek'
        model = profile['model'] || 'deepseek-chat'
        
        # Obtenemos la clave secreta del Keyring según el proveedor
        api_key = fetch_api_key_for(provider)

        {
          provider: provider,
          model: model,
          temperature: profile['temperature'] || 0.2,
          max_tokens: profile['max_tokens'] || 2048,
          api_key: api_key
        }
      end

      # Método principal para despachar una consulta al proveedor configurado
      def dispatch(prompt, options = {})
        target = active_target

        raise "No se encontró una clave API válida para el proveedor: #{target[:provider]}" if target[:api_key].nil? || target[:api_key].empty?

        # Aquí conectaremos los adaptadores específicos de cada proveedor (HTTP client)
        {
          status: :ready,
          target: target,
          prompt: prompt,
          message: "Enrutado con éxito al modelo #{target[:model]} usando el proveedor #{target[:provider]}"
        }
      end

      private

      def fetch_api_key_for(provider)
        # Consultamos el Keyring usando el esquema de atributos adaptado
        case provider.downcase
        when 'deepseek'
          @keyring.get_secret('deepseek_api_key') || @keyring.get_secret('deepseek')
        when 'google', 'gemini'
          @keyring.get_secret('google_api_key') || @keyring.get_secret('gemini')
        when 'openai'
          @keyring.get_secret('openai_api_key') || @keyring.get_secret('openai')
        else
          @keyring.get_secret(provider)
        end
      rescue StandardError => e
        raise "Error al recuperar la credencial para #{provider} desde el Keyring: #{e.message}"
      end
    end
  end
end