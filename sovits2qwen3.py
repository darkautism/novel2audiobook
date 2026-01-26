import os
import re
import shutil
from pathlib import Path
from gradio_client import Client, handle_file

# ================= 設定區 =================
# 你的音訊檔案來源根目錄
SOURCE_DIR = r"E:\GPT-SoVITS-1007-cu124\models\v4"

# 輸出的 .pt 檔案存放目錄
OUTPUT_DIR = "qwen3_tts_voices"

# Gradio API 地址
API_URL = "http://127.0.0.1:8000/"

# 情緒對照表 (中文 -> 英文)
EMOTION_MAP = {
    "中立": "neutral",
    "开心": "happy",
    "生气": "angry",
    "难过": "sad",
    "吃惊": "surprise",
    "恐惧": "fear",
    "厌恶": "disgust",
    "其他": "other"
}
# =========================================

def main():
    # 1. 確保輸出目錄存在
    if not os.path.exists(OUTPUT_DIR):
        os.makedirs(OUTPUT_DIR)
        print(f"建立輸出目錄: {OUTPUT_DIR}")

    # 2. 初始化 Gradio Client
    try:
        client = Client(API_URL)
        print(f"成功連接 API: {API_URL}")
    except Exception as e:
        print(f"無法連接 API，請確認服務已啟動: {e}")
        return

    # 3. 遍歷目錄
    print("開始掃描檔案...")
    
    count_success = 0
    count_fail = 0

    for root, dirs, files in os.walk(SOURCE_DIR):
        for file in files:
            if file.lower().endswith(".wav"):
                file_path = os.path.join(root, file)
                
                # === A. 解析 WAV 檔名獲取情緒與文本 ===
                # 格式範例: 【开心】蕉蕉蕉.wav
                match = re.match(r"^【(.*?)】(.*)\.wav$", file)
                
                if not match:
                    print(f"[略過] 檔名格式不符: {file}")
                    continue
                
                emotion_zh = match.group(1) # 中文情緒
                ref_text = match.group(2)   # REF文字

                # 將中文情緒轉換為英文，如果找不到對應則預設為 'unknown'
                emotion_en = EMOTION_MAP.get(emotion_zh, "unknown")

                # === B. 解析資料夾名稱獲取角色資訊 ===
                try:
                    # 取得相對於 SOURCE_DIR 的第一層資料夾名
                    # 範例資料夾名: "星穹铁道-中文-「蕉授」"
                    rel_path = Path(file_path).relative_to(SOURCE_DIR)
                    folder_name = rel_path.parts[0] 

                    # 解析資料夾結構
                    # 預期格式: 系列名-語言-角色名 (以 "-" 分隔)
                    folder_parts = folder_name.split('-')
                    
                    if len(folder_parts) >= 3 and folder_parts[1] == '中文':
                        series_name = folder_parts[0] # 星穹铁道
                        role_name = folder_parts[2]   # 「蕉授」 (保留原括號，或可視需求移除)
                        
                        # 組合成 "名子" 部分 (用 _ 分隔)
                        # 結果: 星穹铁道_「蕉授」
                        full_name = f"{series_name}_{role_name}"
                    else:
                        # 格式不符時的備案
                        full_name = folder_name.replace('-', '_')
                        print(f"[警告] 資料夾格式特殊，將直接使用: {full_name}")

                except Exception as e:
                    print(f"[錯誤] 無法解析路徑: {file_path}, {e}")
                    continue

                # === C. 呼叫 API ===
                print(f"處理中: {full_name} | {emotion_zh} -> {emotion_en}")
                
                try:
                    result = client.predict(
                        ref_aud=handle_file(file_path),
                        ref_txt=ref_text,
                        use_xvec=False,
                        api_name="/save_prompt"
                    )
                    
                    generated_file_path = result[0]
                    
                    if generated_file_path and os.path.exists(generated_file_path):
                        # === D. 重新命名並搬移 ===
                        # 目標格式: zh-名子-emotion.pt
                        # 範例: zh-星穹铁道_「蕉授」-happy.pt
                        
                        new_filename = f"zh-{full_name}-{emotion_en}.pt"
                        destination = os.path.join(OUTPUT_DIR, new_filename)
                        
                        # 如果檔案已存在，先刪除舊的 (shutil.move 在某些系統覆蓋會有問題)
                        if os.path.exists(destination):
                            os.remove(destination)

                        shutil.move(generated_file_path, destination)
                        print(f"  -> 成功儲存: {new_filename}")
                        count_success += 1
                    else:
                        print(f"  -> API 回傳成功但找不到檔案")
                        count_fail += 1

                except Exception as e:
                    print(f"  -> API 呼叫失敗: {e}")
                    count_fail += 1

    print("------------------------------------------------")
    print(f"處理完成。成功: {count_success}, 失敗: {count_fail}")
    print(f"檔案已儲存於: {os.path.abspath(OUTPUT_DIR)}")

if __name__ == "__main__":
    main()