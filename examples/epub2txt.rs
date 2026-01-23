use epub::doc::EpubDoc;
use std::fs;
use std::io::Write;
use std::path::Path;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let epub_path = "test.epub";
    let output_dir = "input_chapters";

    // 1. 開啟 Epub 檔案
    // 使用 ? 運算子來簡化錯誤處理，比 unwrap() 更安全
    let mut doc = EpubDoc::new(epub_path).expect("無法開啟 epub 檔案");

    // 2. 建立輸出資料夾 (若不存在則建立)
    fs::create_dir_all(output_dir)?;
    println!("正在將內容輸出至 '{}' 資料夾...", output_dir);

    // 3. 處理封面 (Cover)
    // 使用 if let 語法處理 Option，更簡潔
    if let Some((cover_data, mimetype)) = doc.get_cover() {
        let ext = match mimetype.as_str() {
            "image/png" => "png",
            "image/jpeg" | "image/jpg" => "jpg",
            _ => "img",
        };

        let cover_filename = format!("cover.{}", ext);
        let cover_path = Path::new(output_dir).join(cover_filename);
        let mut f = fs::File::create(&cover_path)?;
        f.write_all(&cover_data)?;
        println!("已儲存封面: {:?}", cover_path);
    }

    // 4. 遍歷所有章節並轉存為 TXT
    while doc.go_next() {
        let filename = if let Some(id) = doc.get_current_id() {
            if id == "title" || id == "colophon" || id == "contents" {
                continue; // 跳過標題、序言和目錄
            }
            match extract_number(&id) {
                // 情境 A: 偵測到數字 (例如 "item13" -> 13) -> 轉成 "00013.txt"
                Some(num) => format!("{:05}.txt", num),

                // 情境 B: 沒數字 (例如 "preface") -> "不理" (保持原名 "preface.txt")
                // 如果你的 "不理" 是指 "完全不存檔"，請把這裡改成 continue;
                None => format!("{}.txt", &id),
            }
        } else {
            continue; // 如果沒有 ID，跳過該章節
        };

        // 取得當前章節的 HTML 內容
        // 有些章節可能是空的或無法讀取，我們給予默認空字串
        let (content, mimetype) = doc.get_current_str().unwrap_or_default();
        match mimetype.as_str() {
            "application/xhtml+xml" | "text/html" => {
                // 寫入檔案
                let text_content = html2text::from_read(content.as_bytes(), 500).unwrap();
                let file_path = Path::new(output_dir).join(&filename);
                let mut f = fs::File::create(&file_path)?;
                f.write_all(text_content.as_bytes())?;
            }
            _ => {
                println!("跳過非 HTML 內容的章節，MIME 類型: {}", mimetype);
            }
        }
    }

    println!("轉換完成！");
    Ok(())
}

// 輔助函式：從字串中提取第一組連續的數字
// 例如: "item10" -> Some(10)
//      "part_05_sec" -> Some(5)
//      "intro" -> None
fn extract_number(s: &str) -> Option<u32> {
    // 1. 跳過前面的非數字字符
    // 2. 取出連續的數字字符
    let num_str: String = s
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(|c| c.is_ascii_digit())
        .collect();

    if num_str.is_empty() {
        None
    } else {
        num_str.parse::<u32>().ok()
    }
}
