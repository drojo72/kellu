# frozen_string_literal: true

require 'open3'

module Kellu
  module Security
    class KeyringManager
      def initialize(app_name = 'kellu')
        @app_name = app_name
      end

      def set_secret(service_name, secret_value, category: 'other', description: '')
        # Formato de label estandarizado que el parser lee perfectamente: "Kellu: [descripción o servicio]"
        # Si hay descripción, la incluimos al final para que el label sea descriptivo.
        label_text = description.empty? ? service_name : "#{description} #{service_name}"
        label = "Kellu: #{label_text}"

        command = [
          "secret-tool", "store",
          "--label=#{label}",
          "application", @app_name,
          "service", service_name,
          "category", category
        ]

        stdout, stderr, status = Open3.capture3(*command, stdin_data: secret_value)

        unless status.success?
          raise "Error al guardar en el Keyring: #{stderr.strip}"
        end

        true
      end

      def get_secret(service_name)
        stdout, _, status = Open3.capture3("secret-tool", "lookup", "application", @app_name, "service", service_name)
        return nil unless status.success?
        val = stdout.strip
        val.empty? ? nil : val
      end

      def list_secrets(category: nil)
        command = ["secret-tool", "search", "--all", "application", @app_name]
        command += ["category", category] if category

        stdout, stderr, status = Open3.capture3(*command)
        return [] unless status.success? && !stdout.empty?

        parse_secret_tool_output(stdout)
      end

      private

      def parse_secret_tool_output(output)
        items = []
        current_item = {}

        output.each_line do |line|
          line = line.strip
          next if line.empty?

          if line.match?(/^\[\/\d+\]$/)
            items << current_item unless current_item.empty?
            current_item = {}
            next
          end

          if line.include?('=')
            key, val = line.split('=', 2)
            clean_key = key.strip

            if clean_key == 'label'
              current_item['label'] = val.strip
              # Extraemos el servicio de la última palabra del label (ej: "Kellu: ... cloudflare_r2_token" -> "cloudflare_r2_token")
              parts = val.strip.split(':')
              if parts.length > 1
                current_item['service'] = parts.last.strip.split.last
              else
                current_item['service'] = val.strip.split.last
              end
            elsif clean_key == 'attribute.category'
              current_item['category'] = val.strip
            end
          end
        end
        items << current_item unless current_item.empty?

        # Filtramos para asegurarnos de que tengan un servicio válido
        valid = items.select { |item| item['service'] && !item['service'].empty? }

        # Asignamos categoría por defecto si falta
        valid.each do |item|
          item['category'] ||= classify_category(item['service'])
        end

        valid
      end

      def classify_category(name)
        return 'other' unless name
        case name
        when /ai_|deepseek|google|openrouter|grok|gemma/ then 'ai'
        when /supabase|neon|db/ then 'db'
        when /desec|domain|dns|tunnel/ then 'domain'
        when /cloudflare|render|github|token/ then 'infra'
        else 'other'
        end
      end
    end
  end
end
