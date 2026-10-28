# frozen_string_literal: true

require_relative 'base'

module Kellu
  module Providers
    class DeepSeekAdapter < BaseAdapter
      def complete(prompt, model:, options: {})
        url = "#{@api_base || 'https://api.deepseek.com/v1'}/chat/completions"
        headers = {
          'Content-Type' => 'application/json',
          'Authorization' => "Bearer #{@api_key}"
        }

        body = {
          model: model,
          messages: [{ role: 'user', content: prompt }],
          temperature: options[:temperature] || 0.7
        }

        response = post_request(url, headers, body, options)
        extract_content(response)
      end

      private

      def extract_content(response)
        response.dig('choices', 0, 'message', 'content') || ''
      end
    end
  end
end
