require_relative 'lib/my_gem/version'

Gem::Specification.new do |spec|
  spec.name          = 'my_gem'
  spec.version       = MyGem::VERSION
  spec.authors       = ['Ada Lovelace']
  spec.email         = ['ada@example.com']
  spec.summary       = 'A small gem that does useful things.'
  spec.description   = 'A small gem that does useful things, described at length.'
  spec.homepage      = 'https://example.com/my_gem'
  spec.license       = 'MIT'
  spec.required_ruby_version = '>= 3.0.0'

  spec.files         = Dir.chdir(__dir__) { Dir['lib/**/*.rb', 'README.md'] }
  spec.require_paths = ['lib']

  spec.metadata['source_code_uri'] = 'https://github.com/example/my_gem'

  spec.add_dependency 'json', '~> 2.6'
  spec.add_dependency 'yaml', '>= 0.2'
  spec.add_development_dependency 'rake', '~> 13.0'
end