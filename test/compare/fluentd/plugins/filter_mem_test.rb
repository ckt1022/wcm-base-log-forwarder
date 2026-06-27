require 'fluent/plugin/filter'

module Fluent::Plugin
  class OomFilter < Filter
    Fluent::Plugin.register_filter('mem_test', self)

    config_param :chunk_size_mb, :integer, default: 5
    config_param :max_per_sec,   :integer, default: 5
    config_param :delay_sec,     :integer, default: 5   # 啟動後等幾秒才開始洩漏，方便你確認 worker 真的活著

    def start
      super
      @cache = {}
      @count = 0
      @start_time = Fluent::Engine.now

      log.warn "[MEM TEST] 將於 #{@delay_sec} 秒後開始洩漏，速率 #{max_per_sec} rec/s × #{chunk_size_mb}MB"

      # 用獨立 thread，不依賴任何 input 資料流是否經過這個 filter
      @thread = Thread.new do
        sleep @delay_sec
        loop do
          @count += 1
          @cache["k#{@count}"] = "x" * (chunk_size_mb * 1024 * 1024)
          if (@count % 10).zero?
            rss_mb = `ps -o rss= -p #{Process.pid}`.to_i / 1024
            log.warn "[MEM TEST] count=#{@count} rss=#{rss_mb}MB"
          end
          sleep(1.0 / max_per_sec)
        end
      end
    end

    def shutdown
      @thread&.kill
      super
    end

    def filter(tag, time, record)
      record  # 不再依賴這個方法本身來觸發，純粹是資料流通道
    end
  end
end