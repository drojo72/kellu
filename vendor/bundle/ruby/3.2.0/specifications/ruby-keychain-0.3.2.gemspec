# -*- encoding: utf-8 -*-
# stub: ruby-keychain 0.3.2 ruby lib

Gem::Specification.new do |s|
  s.name = "ruby-keychain".freeze
  s.version = "0.3.2"

  s.required_rubygems_version = Gem::Requirement.new(">= 0".freeze) if s.respond_to? :required_rubygems_version=
  s.require_paths = ["lib".freeze]
  s.authors = ["Frederick Cheung".freeze]
  s.date = "2015-07-09"
  s.description = "Ruby wrapper for OS X's keychain ".freeze
  s.email = "frederick.cheung@gmail.com".freeze
  s.homepage = "http://github.com/fcheung/keychain".freeze
  s.licenses = ["MIT".freeze]
  s.required_ruby_version = Gem::Requirement.new(">= 1.9.2".freeze)
  s.rubygems_version = "3.4.20".freeze
  s.summary = "Ruby wrapper for  OS X's keychain".freeze

  s.installed_by_version = "3.4.20" if s.respond_to? :installed_by_version

  s.specification_version = 4

  s.add_runtime_dependency(%q<ffi>.freeze, [">= 0"])
  s.add_runtime_dependency(%q<corefoundation>.freeze, ["~> 0.2.0"])
  s.add_development_dependency(%q<rspec>.freeze, ["~> 3.3", ">= 3.3.0"])
  s.add_development_dependency(%q<rake>.freeze, ["~> 10.4", ">= 10.4.2"])
  s.add_development_dependency(%q<yard>.freeze, ["~> 0.8.7"])
  s.add_development_dependency(%q<redcarpet>.freeze, ["~> 3.2", ">= 3.2.3"])
end
