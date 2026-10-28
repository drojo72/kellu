# frozen_string_literal: true

require "bundler/setup"
 Bundler.require(:default)

module Kellu
  VERSION = "0.1.0"
  class Error < StandardError; end
end

require_relative "kellu/security/keyring"
require_relative "kellu/core/config"
require_relative "kellu/core/router"

# Requerimos el módulo de proveedores y sus adaptadores
require_relative "kellu/providers/base"
require_relative "kellu/providers/deepseek_adapter"
require_relative "kellu/providers/openrouter_adapter"
require_relative "kellu/providers/google_adapter"
require_relative "kellu/providers/gemma_adapter"
