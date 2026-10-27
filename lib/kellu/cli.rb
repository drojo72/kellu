# frozen_string_literal: true

require "thor"
require_relative 'core/config'
require_relative 'core/router'
require_relative 'security/keyring'

module Kellu
  class SecretCLI < Thor
    desc "set [SERVICE]", "Guarda o actualiza una credencial"
    option :category, type: :string, desc: "Categoría (ai, db, domain, infra, other)"
    option :description, type: :string, desc: "Descripción amigable"
    def set(service = nil)
      service ||= ask("Nombre del servicio:", echo: true)
      category = options[:category] || "other"
      description = options[:description] || ""
      secret_value = ask("Clave o token:", echo: false)
      say ""

      manager = Kellu::Security::KeyringManager.new
      manager.set_secret(service, secret_value, category: category, description: description)
      say "✅ Credencial '#{service}' guardada con éxito.", :green
    end

    desc "get [SERVICE]", "Recupera y muestra una credencial por su nombre de servicio"
    def get(service)
      manager = Kellu::Security::KeyringManager.new
      value = manager.get_secret(service)

      if value
        say value
      else
        say "⚠️ Credencial '#{service}' no encontrada en el Keyring.", :red
        exit(1)
      end
    end

    desc "list", "Lista credenciales, permite borrarlas o añadir nuevas de forma interactiva"
    option :category, aliases: "-c", type: :string, desc: "Filtra por categoría específica (ai, db, domain, infra, other)"
    def list
      manager = Kellu::Security::KeyringManager.new

      loop do
        secrets = manager.list_secrets

        if secrets.empty?
          say "No se encontraron credenciales en el Keyring.", :yellow
          # Si está vacío, permitimos al menos añadir una clave
          choice = ask("¿Deseas crear una nueva clave ahora? (+ para crear, o presiona Enter para salir):", default: "")
          break unless choice == "+"
          interactive_create(manager)
          next
        end

        # Aplicar filtro de categoría si el usuario lo indicó
        filter_category = options[:category]&.downcase
        display_secrets = secrets
        if filter_category
          display_secrets = secrets.select { |s| (s['category'] || 'other').downcase == filter_category }
          if display_secrets.empty?
            say "No se encontraron credenciales para la categoría '#{filter_category}'.", :yellow
            break
          end
        end

        # 1. Agrupamos por categoría
        categorized = Hash.new { |h, k| h[k] = [] }
        display_secrets.each do |secret|
          cat = secret['category'] || 'other'
          categorized[cat] << secret
        end

        # 2. Asignamos índice global para visualización
        indexed_secrets = []
        current_index = 1

        say "\n🔑 Credenciales almacenadas:", :green
        categorized.each do |cat_key, items|
          say "\n[#{cat_key.upcase}]:", :cyan
          items.each do |item|
            item_with_index = item.merge('index' => current_index)
            indexed_secrets << item_with_index

            label_info = item['label'] ? " - #{item['label']}" : ""
            say "  [#{current_index}] #{item['service']}#{label_info}"
            current_index += 1
          end
        end

        say "\n----------------------------------------"
        prompt_msg = "Selecciona el número a borrar, [+] para crear nueva clave, o Enter para salir:"
        choice = ask(prompt_msg, default: "salir").to_s.strip

        break if choice.downcase == "salir" || choice.empty?

        if choice == "+"
          interactive_create(manager)
          next
        end

        selected_index = choice.to_i
        target_secret = indexed_secrets.find { |s| s['index'] == selected_index }

        unless target_secret
          say "⚠️ Opción o número inválido.", :red
          sleep(1)
          next
        end

        service_name = target_secret['service'].to_s.strip
        category_name = (target_secret['category'] || 'other').to_s.strip

        response = ask("¿Estás seguro de que deseas eliminar permanentemente la credencial '#{service_name}'? [s/N]:", default: "n").to_s.strip

        if response.downcase == "s" || response.downcase == "si"
          # Pasamos los atributos completos que exige secret-tool clear para encontrar el registro exacto
          command = [
            "secret-tool", "clear",
            "application", "kellu",
            "service", service_name,
            "category", category_name
          ]
          _, stderr, status = Open3.capture3(*command)

          if status.success?
            say "✅ Credencial '#{service_name}' eliminada con éxito.", :green
          else
            say "❌ Error al eliminar del Keyring: #{stderr.strip}", :red
          end
          sleep(1)
        else
          say "Operación cancelada.", :yellow
          sleep(1)
        end
      end
    end

    private

    def interactive_create(manager)
      say "\n✨ --- Nueva Credencial ---", :magenta

      # Limpiamos estrictamente el nombre del servicio para evitar espacios o saltos de línea
      service = ask("Nombre del servicio:", echo: true).to_s.strip
      return if service.empty?

      category = ask("Categoría (ai, db, domain, infra, other) [other]:", default: "other").to_s.strip
      description = ask("Descripción amigable:", echo: true).to_s.strip
      secret_value = ask("Clave o token:", echo: false).to_s.strip
      say ""

      begin
        manager.set_secret(service, secret_value, category: category, description: description)
        say "✅ Credencial '#{service}' guardada y sincronizada con éxito.", :green
      rescue => e
        say "❌ Error al guardar: #{e.message}", :red
      end
      sleep(1)
    end
  end

  class CLI < Thor
    def self.exit_on_failure?
      true
    end

    desc "secret SUBCOMMAND ...", "Gestiona las credenciales de forma segura"
    subcommand "secret", SecretCLI

    desc "version", "Muestra la versión actual"
    def version
      say "Kellu v#{Kellu::VERSION}", :cyan
    end
  end
end
