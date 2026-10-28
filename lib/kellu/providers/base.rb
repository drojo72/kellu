# frozen_string_literal: true

require 'net/http'
require 'uri'
require 'json'

module Kellu
  module Providers
    class BaseAdapter
      attr_reader :api_key, :api_base

      def initialize(api_key:, api_base: nil)
        @api_key = api_key
        @api_base = api_base
      end

      def complete(prompt, model:, options: {})
        raise NotImplementedError, "Las subclases deben implementar el método #complete"
      end

      protected

      def post_request(url, headers, body, options = {})
        uri = URI.parse(url)
        http = Net::HTTP.new(uri.host, uri.port)
        http.use_ssl = (uri.scheme == 'https')
        http.read_timeout = options_timeout(options)

        request = Net::HTTP::Post.new(uri.request_uri, headers)
        request.body = body.to_json

        response = http.request(request)
        handle_response(response)
      end

      def handle_response(response)
        unless response.is_a?(Net::HTTPSuccess)
          raise "Error en la API del proveedor (#{response.code}): #{response.body}"
        end

        JSON.parse(response.body)
      end

      def options_timeout(options)
        options[:timeout] || 30
      end
    end
  end
end
