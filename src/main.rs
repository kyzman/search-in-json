extern crate flate2;

use clap::Parser;
use flate2::read::GzDecoder;
use serde_json;
use serde_json::Value;
use std::fs::File;
use std::io::{IsTerminal, Read, stdout};
use std::time::Instant;

#[derive(Parser, Debug, Clone)]
#[command(name = "search")]
struct Args {
    #[arg(short = 'f', long = "file")]
    file: String,

    #[arg(short = 's', long = "search")]
    search_query: String,
}

fn main() -> Result<(), std::io::Error> {
    let args = Args::parse();
    print!("Загрузка и распаковка JSON...");
    // let path = "archive_90.json.gz";
    let path = args.file;
    let start = Instant::now();
    let tar_gz = File::open(path)?;
    let mut tar = GzDecoder::new(tar_gz);
    let mut buf = String::new();
    tar.read_to_string(&mut buf)?;
    let mut out: Value = serde_json::from_str(&buf)?;
    clean_json_value(&mut out); // Очистка JSON для корректного отображения и поиска
    // 2. Что мы ищем (работает и для кириллицы, и для латиницы)
    // let search_query = "параметры"; // Или "ScriptRunner", или "display_value"
    // let search_query = "epaklina@monetka.ru"; // Или "ScriptRunner", или "display_value"
    let elapsed = start.elapsed();
    println!("{:.5} сек.", elapsed.as_secs_f64());

    let start = Instant::now();
    println!("Результаты поиска для: \"{}\"\n", args.search_query);

    // 3. Запуск поиска от корня ("$")
    search_in_json(&out, &args.search_query, "$");
    // println!("{}", serde_json::to_string_pretty(&out)?);
    // let mut archive = Archive::new(tar);
    // archive.unpack(".")?;
    let elapsed = start.elapsed();
    println!("--- Поиск занял {:.5} сек.", elapsed.as_secs_f64());
    Ok(())
}

fn clean_json_value(value: &mut Value) {
    match value {
        // Если это строка, пробуем распарсить её как внутренний JSON
        Value::String(s) => {
            // Если строка валидна как JSON (например, содержит внутренний объект или массив)
            if let Ok(inner_value) = serde_json::from_str::<Value>(s) {
                let mut decoded_inner = inner_value;
                // Рекурсивно чистим то, что было внутри этой строки
                clean_json_value(&mut decoded_inner);
                *value = decoded_inner;
            }
        }
        // Если это массив, чистим каждый элемент
        Value::Array(arr) => {
            for item in arr {
                clean_json_value(item);
            }
        }
        // Если это объект, чистим каждое значение по ключу
        Value::Object(obj) => {
            for (_key, val) in obj.iter_mut() {
                clean_json_value(val);
            }
        }
        _ => {}
    }
}

fn search_in_json(value: &Value, query: &str, current_path: &str) {
    let query_lowercase = query.to_lowercase();

    match value {
        // Если элемент — это JSON-объект (Map)
        Value::Object(obj) => {
            for (key, val) in obj {
                // Строим путь для текущего ключа
                let next_path = format!("{}.{}", current_path, key);

                // А. Проверяем, совпадает ли сам КЛЮЧ
                if key.to_lowercase().contains(&query_lowercase) {
                    println!("[Найдено в КЛЮЧЕ]");
                    println!("Путь: {}", next_path);
                    println!("Значение по этому ключу: {}\n", truncate_value(val));
                }

                // Б. Идем глубже в значение по этому ключу
                search_in_json(val, query, &next_path);
            }
        }
        // Если элемент — это массив
        Value::Array(arr) => {
            for (index, item) in arr.iter().enumerate() {
                // Строим путь с индексом массива, например: $.key[0]
                let next_path = format!("{}[{}]", current_path, index);
                search_in_json(item, query, &next_path);
            }
        }
        // Если элемент — это строка (Значение)
        Value::String(s) => {
            // В. Проверяем, совпадает ли ЗНАЧЕНИЕ
            if s.to_lowercase().contains(&query_lowercase) {
                println!("[Найдено в ЗНАЧЕНИИ]");
                println!("Путь: {}", current_path);
                println!("Текст: \"{}\"\n", s);
            }
        }
        // Числа, булевы значения и null игнорируем (или можно добавить при желании)
        _ => {}
    }
}

// Вспомогательная функция, чтобы не выводить слишком огромные объекты в консоль при совпадении ключа
fn truncate_value(val: &Value) -> String {
    let s = serde_json::to_string(val).unwrap_or_default();
    if s.len() > 150 {
        format!("{}... [обрезано]", &s[..150])
    } else {
        s
    }
}
