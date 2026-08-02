require 'json'
require 'yaml'
require 'uri'

require_relative './local_helper'

# Read and parse a JSON document from disk.
def load_document(path)
  content = File.read(path)
  JSON.parse(content)
end

# Load configuration from a YAML file.
def load_config(path)
  YAML.safe_load(File.read(path))
end

def build_url(host, port)
  base = "http://#{host}:#{port}"
  endpoint(base, '/health')
end

def endpoint(base, path)
  URI.join(base, path).to_s
end

def main
  config = load_config('config.yaml')
  url = build_url(config['host'], config['port'])
  puts url
end