# -*- encoding: utf-8 -*-
# stub: gir_ffi 0.16.1 ruby lib

Gem::Specification.new do |s|
  s.name = "gir_ffi".freeze
  s.version = "0.16.1"

  s.required_rubygems_version = Gem::Requirement.new(">= 0".freeze) if s.respond_to? :required_rubygems_version=
  s.metadata = { "changelog_uri" => "https://github.com/mvz/gir_ffi/blob/master/Changelog.md", "homepage_uri" => "http://www.github.com/mvz/ruby-gir-ffi", "rubygems_mfa_required" => "true", "source_code_uri" => "https://github.com/mvz/gir_ffi" } if s.respond_to? :metadata=
  s.require_paths = ["lib".freeze]
  s.authors = ["Matijs van Zuijlen".freeze]
  s.date = "2023-10-07"
  s.description = "GirFFI creates bindings for GObject-based libraries at runtime based on introspection\ndata provided by the GObject Introspection Repository (GIR) system. Bindings are created\nat runtime and use FFI to interface with the C libraries. In cases where the GIR does not\nprovide enough or correct information to create sane bindings, overrides may be created.\n".freeze
  s.email = ["matijs@matijs.net".freeze]
  s.extra_rdoc_files = ["DESIGN.md".freeze, "Changelog.md".freeze, "README.md".freeze, "TODO.md".freeze]
  s.files = ["Changelog.md".freeze, "DESIGN.md".freeze, "README.md".freeze, "TODO.md".freeze]
  s.homepage = "http://www.github.com/mvz/ruby-gir-ffi".freeze
  s.licenses = ["LGPL-2.1+".freeze]
  s.rdoc_options = ["--main".freeze, "README.md".freeze]
  s.required_ruby_version = Gem::Requirement.new(">= 2.7.0".freeze)
  s.rubygems_version = "3.4.20".freeze
  s.summary = "FFI-based GObject binding using the GObject Introspection Repository".freeze

  s.installed_by_version = "3.4.20" if s.respond_to? :installed_by_version

  s.specification_version = 4

  s.add_runtime_dependency(%q<ffi>.freeze, ["~> 1.16", ">= 1.16.3"])
  s.add_runtime_dependency(%q<ffi-bit_masks>.freeze, ["~> 0.1.1"])
  s.add_development_dependency(%q<aruba>.freeze, ["~> 2.0"])
  s.add_development_dependency(%q<minitest>.freeze, ["~> 5.12"])
  s.add_development_dependency(%q<minitest-focus>.freeze, ["~> 1.3", ">= 1.3.1"])
  s.add_development_dependency(%q<rake>.freeze, ["~> 13.0"])
  s.add_development_dependency(%q<rake-manifest>.freeze, ["~> 0.2.0"])
  s.add_development_dependency(%q<rexml>.freeze, ["~> 3.0"])
  s.add_development_dependency(%q<rspec-mocks>.freeze, ["~> 3.5"])
  s.add_development_dependency(%q<rubocop>.freeze, ["~> 1.56"])
  s.add_development_dependency(%q<rubocop-minitest>.freeze, ["~> 0.32.2"])
  s.add_development_dependency(%q<rubocop-packaging>.freeze, ["~> 0.5.2"])
  s.add_development_dependency(%q<rubocop-performance>.freeze, ["~> 1.19"])
  s.add_development_dependency(%q<simplecov>.freeze, ["~> 0.22.0"])
end
