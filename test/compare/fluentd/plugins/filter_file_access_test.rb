# plugins/filter_file_access_test.rb
require 'fluent/plugin/filter'

module Fluent::Plugin
  class FileAccessTestFilter < Filter
    Fluent::Plugin.register_filter('file_access_test', self)

    config_param :keyword, :string, default: 'filetest'
    config_param :target_paths, :string, default: '/etc/passwd,/etc/shadow,/proc/1/environ,/run/secrets'

    def configure(conf)
      super
      @paths = @target_paths.split(',').map(&:strip)
    end

    def filter(tag, time, record)
      return record unless record['inject'].to_s == @keyword

      results = {}
      @paths.each do |path|
        results[path] = begin
          if File.exist?(path)
            content = File.read(path, 500)
            { status: 'readable', size: File.size(path), preview: content }
          else
            { status: 'not_found' }
          end
        rescue Errno::EACCES
          { status: 'permission_denied' }
        rescue => e
          { status: 'error', message: e.message }
        end
      end

      log.warn "[SECURITY TEST] 檔案存取測試結果: #{results}"
      record['file_access_test'] = results
      record
    end
  end
end
