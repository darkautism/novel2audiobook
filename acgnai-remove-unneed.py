import json
import os

def strict_filter_json(input_file, output_file):
    print(f"正在讀取 {input_file}...")
    try:
        with open(input_file, 'r', encoding='utf-8') as f:
            data = json.load(f)
    except FileNotFoundError:
        print("錯誤：找不到輸入檔案。")
        return

    original_count = len(data)
    
    # ============================
    # 🔧 設定區 (您可以調整這裡)
    # ============================
    
    # 1. 演技門檻：至少要有幾種 Emotion 才保留？
    # (例如：默認, 開心, 生氣, 難過 這樣算 4 種)
    MIN_EMOTION_COUNT = 6

    # 2. 路人黑名單標籤：只要包含這些標籤的角色就刪除
    # 包含繁簡體常見寫法
    BANNED_TAGS = [
        "普通", "平民", "龙套", "龍套", "路人", "村民", 
        "士兵", "卫兵", "守卫", "男", "女", # 太過籠統的標籤
        "怪物", "生物", "纯水精灵", "元素生命", "丘丘人"
    ]

    # 3. 名字黑名單 (部分名字本身就是雜魚)
    BANNED_NAMES = [
        "NPC", "系统", "旁白", "未知", "大叔", "小孩", "少女"
    ]

    # ============================
    # 階段一：預處理 (合併 _ZH 與 非_ZH)
    # ============================
    merged_data = {}
    # 先把所有 key 轉成不帶 _ZH 的基礎名，用來判斷重複
    # 邏輯：優先存入帶 _ZH 的資料，如果遇到不帶 _ZH 的，只有在沒資料時才存入
    
    # 為了確保 _ZH 優先，我們先處理所有帶 _ZH 的 keys
    sorted_keys = sorted(data.keys(), key=lambda k: 1 if k.endswith('_ZH') else 2)
    
    temp_map = {} # map[base_name] = full_key

    for key in sorted_keys:
        value = data[key]
        
        # 語言過濾 (雖然你的新JSON可能已經沒這些了，但保留著以防萬一)
        key_lower = str(key).lower()
        if any(x in key_lower for x in ["_en", "_ja", "english", "japanese", "英语", "日语"]):
            continue

        base_name = key.replace("_ZH", "")
        
        if base_name in temp_map:
            # 已存在 (因為我們讓 _ZH 優先跑，所以這裡通常是遇到了無 _ZH 的版本)
            # 我們把無 _ZH 版本的 tags 合併進去，但保留 _ZH 的主體數據
            existing_key = temp_map[base_name]
            existing_data = merged_data[existing_key]
            
            # 合併 Tags
            new_tags = set(existing_data.get('tags', [])) | set(value.get('tags', []))
            merged_data[existing_key]['tags'] = list(new_tags)
        else:
            # 新條目
            temp_map[base_name] = key
            merged_data[key] = value

    print(f"預處理(去重/語言過濾)後數量: {len(merged_data)}")

    # ============================
    # 階段二：高強度過濾
    # ============================
    final_data = {}
    
    for key, value in merged_data.items():
        # 1. 檢查 Emotion 數量 [修改點：適應新結構]
        # 直接讀取 emotion list，如果沒有該 key 則回傳空 list
        emotions = value.get("emotion", [])
        
        # 簡單的防呆，以防萬一有些舊數據沒改到
        if not isinstance(emotions, list):
            # 如果不是 list (例如還是舊的 dict)，嘗試抓取值
            if isinstance(emotions, dict):
                 emotions = list(emotions.values())[0] if emotions else []
        
        if len(emotions) < MIN_EMOTION_COUNT:
            continue

        # 2. 檢查黑名單標籤
        current_tags = value.get("tags", [])
        is_banned_tag = False
        for tag in current_tags:
            for banned in BANNED_TAGS:
                if banned in tag: # 例如 "普通人" 包含 "普通"
                    is_banned_tag = True
                    is_banned_tag = True
                    break
            if is_banned_tag: break
        
        if is_banned_tag:
            continue

        # 3. 檢查名字黑名單
        is_banned_name = False
        for bad_name in BANNED_NAMES:
            # key 格式通常是 "原神-中文-名字_ZH" 或 "原神-中文-名字"
            # 取最後一段並去掉 _ZH
            name_part = key.split('-')[-1].replace("_ZH", "")
            
            if bad_name == name_part: 
                is_banned_name = True
                break
        
        if is_banned_name:
            continue

        final_data[key] = value

    # ============================
    # 輸出
    # ============================
    removed_count = original_count - len(final_data)
    print("-" * 30)
    print(f"高強度清洗完成。")
    print(f"原始數量: {original_count}")
    print(f"最終數量: {len(final_data)}")
    print(f"共移除: {removed_count}")

    with open(output_file, 'w', encoding='utf-8') as f:
        json.dump(final_data, f, ensure_ascii=False, indent=2)
    print(f"檔案已儲存: {output_file}")

if __name__ == "__main__":
    # 請確保這裡的檔名是你最新的檔案
    input_filename = 'acgnai-voice.json' 
    output_filename = 'acgnai-voice-elite.json'
    
    if os.path.exists(input_filename):
        strict_filter_json(input_filename, output_filename)
    else:
        print(f"找不到輸入檔案: {input_filename}")