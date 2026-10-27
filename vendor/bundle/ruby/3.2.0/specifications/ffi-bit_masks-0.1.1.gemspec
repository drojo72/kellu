# -*- encoding: utf-8 -*-
# stub: ffi-bit_masks 0.1.1 ruby lib

Gem::Specification.new do |s|
  s.name = "ffi-bit_masks".freeze
  s.version = "0.1.1"

  s.required_rubygems_version = Gem::Requirement.new(">= 0".freeze) if s.respond_to? :required_rubygems_version=
  s.require_paths = ["lib".freeze]
  s.authors = ["Postmodern".freeze]
  s.date = "2016-03-09"
  s.description = "FFI plugin which adds support for bitmasked types (or flags) to FFI.".freeze
  s.email = "postmodern.mod3@gmail.com".freeze
  s.extra_rdoc_files = ["ChangeLog.md".freeze, "LICENSE.txt".freeze, "README.md".freeze]
  s.files = ["ChangeLog.md".freeze, "LICENSE.txt".freeze, "README.md".freeze]
  s.homepage = "https://github.com/postmodern/ffi-bit_masks#readme".freeze
  s.licenses = ["MIT".freeze]
  s.rubygems_version = "3.4.20".freeze
  s.summary = "Adds support for bit-masked types in FFI.".freeze

  s.installed_by_version = "3.4.20" if s.respond_to? :installed_by_version

  s.specification_version = 4

  s.add_runtime_dependency(%q<ffi>.freeze, ["~> 1.0"])
  s.add_development_dependency(%q<rspec>.freeze, ["~> 3.0"])
  s.add_development_dependency(%q<rubygems-tasks>.freeze, ["~> 0.2"])
  s.add_development_dependency(%q<yard>.freeze, ["~> 0.8"])
  s.add_development_dependency(%q<rake>.freeze, ["~> 10.4"])
end
