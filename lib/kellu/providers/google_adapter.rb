# frozen_string_literal: true

require_relative 'base'

module Kellu
  module Providers
    class GoogleAdapter < BaseAdapter
      def complete(prompt, model:, options: {})
        # Ejemplo para Google Generative Language API
        target_model = model || 'gemini-2.5-pro'
        url = "https://generativelanguage.googleapis.com/v1beta/models/#{target_model}:generateContent?key=#{@api_key}"
        
        headers = {
          'Content-Type' => 'application/json'
        }

        body = {
          contents: [
            {
              parts: [
                { text: prompt }
              ]
            }
          ]
        }

        response = post_request(url, headers, body, options)
        extract_google_content(response)
      end

      private

      def extract_google_content(response)
        response.dig('candidates', 0, 'content', 'parts', 0, 'text') || ''
      end
    end
  end
end
