require 'json'
require 'net/http'
require_relative './widget_store'

# The Widget namespace groups all widget-related classes.
module Widget
  VERSION = '1.2.0'.freeze

  # A Report renders widget metrics to text or JSON.
  class Report
    FORMATS = %w[json yaml].freeze

    def initialize(title, rows = [])
      @title = title
      @rows = rows
    end

    # Compute the total value across all rows.
    def total
      @rows.sum
    end

    # Render the report using the configured formatter.
    def render
      format_rows(@rows)
    end

    # Serialize the report to a JSON document.
    def to_json
      JSON.generate(payload)
    end

    # Build a report directly from a local file path.
    def self.from_file(path)
      new(path)
    end

    private

    def payload
      { title: @title, rows: @rows }
    end

    def format_rows(rows)
      rows.map(&:to_s).join("\n")
    end
  end
end

# The Auth namespace wraps the remote auth service.
module Auth
  ENDPOINT = 'https://auth.example.com'.freeze

  # A Token manages an opaque session secret.
  class Token
    def initialize(secret)
      @secret = secret
      @expires_at = Time.now + 3600
    end

    def fresh?
      @expires_at > Time.now
    end

    def self.for(secret)
      new(secret)
    end
  end
end

report = Widget::Report.from_file('data/report.json')
client = HTTP::Client.new(Auth::ENDPOINT)
root = Math.sqrt(report.total)
puts client.inspect
puts root