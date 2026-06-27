# plugins/filter_file_cpu_test.rb
require 'fluent/plugin/filter'

module Fluent::Plugin
  class CpuTestFilter < Filter
    Fluent::Plugin.register_filter('cpu_test', self)

    config_param :keyword, :string, default: 'cputest'
    config_param :worker_threads, :integer, default: 2

    def filter(tag, time, record)
      msg = record['message'].to_s
      return record unless msg.include?(@keyword)

      log.warn "[CPU TEST] 觸發 CPU 耗盡，進入無限數學迴圈（block filter worker）"
      x = 0.0
      loop { x = Math.sqrt(x + 1) }
      record
    end
  end
end
