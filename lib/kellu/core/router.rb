# frozen_string_literal: true

require_relative 'config'
# Suponiendo que KeyringManager se encuentra accesible en el namespace o archivo requerido
# require_relative 'keyring_manager' 

module Kellu
  module Core
    class Router
      attr_reader :config, :keyring

      def initialize(config: nil, keyring: nil)
        @config = config || Config.new
        # @keyring = keyring || KeyringManager.new
      end

      # Envía un mensaje/prompt al proveedor activo o seleccionado
      def chat(prompt, provider_name: nil, model: nil, options: {})
        provider_name ||= resolve_default_provider
        provider_config = @config.provider_config(provider_name)

        raise "El proveedor '#{provider_name}' no está habilitado o no existe en la configuración." unless provider_config['enabled']

        # Resolver modelo (prioriza parámetro explícito, luego config, luego fallback)
        target_model = model || provider_config['model'] || default_model_for(provider_name)
        
        # Obtener credencial segura desde KeyringManager (ej: "kellu_api_key_deepseek")
        api_key = fetch_api_key(provider_name)

        # Delegar la llamada al adaptador correspondiente
        adapter = build_adapter(provider_name, api_key, provider_config)
        adapter.complete(prompt, model: target_model, options: options)
      end

      private

      def resolve_default_provider
        @config.general['default_provider'] || 'deepseek'
      end

      def default_model_for(provider_name)
        case provider_name.to_s
        when 'deepseek' then 'deepseek-chat'
        when 'openrouter' then 'anthropic/claude-3.5-sonnet'
        when 'google' then 'gemini-2.5-pro'
        else 'default-model'
        end
      end

      def fetch_api_key(provider_name)
        provider_config = @config.provider_config(provider_name)

        # Lee el servicio exacto configurado en el TOML (ej: "ai_openrouter_api_key")
        service_name = provider_config['keyring_service'] || provider_name

        manager = Kellu::Security::KeyringManager.new
        key = manager.get_secret(service_name)

        unless key
          env_var = "KELLU_#{provider_name.upcase}_API_KEY"
          key = ENV[env_var]
        end

        unless key
          raise "No se encontró clave API para el servicio '#{service_name}' (proveedor '#{provider_name}') en el llavero seguro ni en variables de entorno."
        end

        key
      end

      def build_adapter(provider_name, api_key, provider_config)
        # En la siguiente subfase implementaremos los adaptadores bajo lib/kellu/providers/
        case provider_name.to_s
        when 'deepseek'
          Kellu::Providers::DeepSeekAdapter.new(api_key: api_key, api_base: provider_config['api_base'])
        when 'openrouter'
          Kellu::Providers::OpenRouterAdapter.new(api_key: api_key, api_base: provider_config['api_base'])
        when 'google'
          Kellu::Providers::GoogleAdapter.new(api_key: api_key)
        else
          raise "Adaptador no soportado para el proveedor: #{provider_name}"
        end
      end
    end
  end
end
