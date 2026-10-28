# frozen_string_literal: true

require_relative 'base'

module Kellu
  module Providers
    class GemmaAdapter < BaseAdapter
      def complete(prompt, model:, options: {})
        target_model = model || 'gemma'
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
        extract_gemma_content(response)
      end

      private

      def extract_gemma_content(response)
        response.dig('candidates', 0, 'content', 'parts', 0, 'text') || ''
      end
    end
  end
end
