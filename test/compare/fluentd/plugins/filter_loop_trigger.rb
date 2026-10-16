# plugins/filter_loop_trigger.rb
require 'fluent/plugin/filter'

module Fluent::Plugin
  class LoopTriggerFilter < Filter
    Fluent::Plugin.register_filter('loop_trigger', self)

    config_param :keyword, :string, default: 'loop'

    def filter(tag, time, record)
      return record unless record['inject'].to_s == @keyword

      log.warn "偵測到 inject='#{@keyword}'，即將進入無窮迴圈"
      loop do
        # 完全不做任何事、不 sleep，CPU 100% 卡死
      end
      record
    end
  end
end
