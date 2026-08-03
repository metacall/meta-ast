# Greet a user by name.
def greet(name)
  puts "Hello, #{name}"
end

def add(a, b)
  a + b
end

def configure(url, port = 8080)
  puts "Connecting to #{url} on port #{port}"
end

# Fetch a path with retry and timeout options.
def fetch(path, retries: 3, timeout: 5)
  retries.times { puts path }
  puts "timeout=#{timeout}"
end

def main
  greet('world')
  add(1, 2)
  configure('localhost')
end