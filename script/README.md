# 這個資料夾存放測試本系統的腳本

## 安全性測試
### 插件出現無窮迴圈
* 使用C-sharp的插件進行無窮迴圈測試。
結果 => 若5秒沒有回應，則重試，重試兩次仍失敗就寫入檔案。
圖片:test-plugins/csharp-plugin/parse_loop/loop.png
### 插件向外存取能力
* 使用C插件進行外部存取
結果 => 成功阻絕 
圖片在/test-plugins/c-plugin/parse_access/access.png
### CPU/MEM 無限上升
* 使用GO進行無限上升測試
感覺不用作
---
## 流量測試