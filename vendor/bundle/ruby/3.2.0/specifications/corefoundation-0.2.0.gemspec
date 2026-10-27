# -*- encoding: utf-8 -*-
# stub: corefoundation 0.2.0 ruby lib

Gem::Specification.new do |s|
  s.name = "corefoundation".freeze
  s.version = "0.2.0"

  s.required_rubygems_version = Gem::Requirement.new(">= 0".freeze) if s.respond_to? :required_rubygems_version=
  s.require_paths = ["lib".freeze]
  s.authors = ["Frederick Cheung".freeze]
  s.date = "2012-12-06"
  s.description = "FFI based Ruby wrappers for Core Foundation ".freeze
  s.email = "frederick.cheung@gmail.com".freeze
  s.homepage = "http://github.com/fcheung/corefoundation".freeze
  s.licenses = ["MIT".freeze]
  s.required_ruby_version = Gem::Requirement.new(">= 1.8.7".freeze)
  s.rubygems_version = "3.4.20".freeze
  s.summary = "Ruby wrapper for OS X corefoundation".freeze

  s.installed_by_version = "3.4.20" if s.respond_to? :installed_by_version

  s.specification_version = 3

  s.add_runtime_dependency(%q<ffi>.freeze, [">= 0"])
  s.add_development_dependency(%q<rspec>.freeze, ["~> 2.10"])
  s.add_development_dependency(%q<rake>.freeze, [">= 0"])
  s.add_development_dependency(%q<yard>.freeze, [">= 0"])
  s.add_development_dependency(%q<redcarpet>.freeze, [">= 0"])
end
