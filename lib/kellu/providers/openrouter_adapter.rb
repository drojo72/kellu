# frozen_string_literal: true

require_relative 'base'

module Kellu
  module Providers
    class OpenRouterAdapter < BaseAdapter
      def complete(prompt, model:, options: {})
        url = "#{@api_base || 'https://openrouter.ai/api/v1'}/chat/completions"
        headers = {
          'Content-Type' => 'application/json',
          'Authorization' => "Bearer #{@api_key}",
          'HTTP-Referer' => 'https://github.com/kellu-cli/kellu',
          'X-Title' => 'Kellu CLI'
        }

        body = {
          model: model,
          messages: [{ role: 'user', content: prompt }],
          temperature: options[:temperature] || 0.7
        }

        response = post_request(url, headers, body, options)
        response.dig('choices', 0, 'message', 'content') || ''
      end
    end
  end
end
