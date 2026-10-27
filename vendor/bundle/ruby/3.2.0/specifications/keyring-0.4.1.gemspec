# -*- encoding: utf-8 -*-
# stub: keyring 0.4.1 ruby lib
# stub: ext/mkrf_conf.rb

Gem::Specification.new do |s|
  s.name = "keyring".freeze
  s.version = "0.4.1"

  s.required_rubygems_version = Gem::Requirement.new(">= 0".freeze) if s.respond_to? :required_rubygems_version=
  s.require_paths = ["lib".freeze]
  s.authors = ["Jason Heiss".freeze]
  s.date = "2016-10-13"
  s.description = "This library provides a easy way to access the system keyring service from ruby".freeze
  s.email = ["jheiss@aput.net".freeze]
  s.executables = ["keyring".freeze]
  s.extensions = ["ext/mkrf_conf.rb".freeze]
  s.files = ["bin/keyring".freeze, "ext/mkrf_conf.rb".freeze]
  s.homepage = "https://github.com/jheiss/keyring".freeze
  s.licenses = ["MIT".freeze]
  s.rubygems_version = "3.4.20".freeze
  s.summary = "Store and access your passwords safely".freeze

  s.installed_by_version = "3.4.20" if s.respond_to? :installed_by_version

  s.specification_version = 4

  s.add_development_dependency(%q<bundler>.freeze, ["~> 1.3"])
  s.add_development_dependency(%q<rake>.freeze, [">= 0"])
  s.add_development_dependency(%q<mocha>.freeze, [">= 0"])
  s.add_runtime_dependency(%q<slop>.freeze, ["< 4.0"])
  s.add_runtime_dependency(%q<ruby-keychain>.freeze, ["~> 0.3.2"])
end
