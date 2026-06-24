extern crate flate2;

use clap::Parser;
use flate2::read::GzDecoder;
use glob::glob;
use serde_json;
use serde_json::Value;
use std::fs::File;
use std::io::{IsTerminal, Read, stdout};
use std::path::PathBuf;
use std::time::Instant;

#[derive(Parser, Debug, Clone)]
#[command(name = "search")]
struct Args {
    #[arg(short = 'f', long = "file")]
    file: String,

    #[arg(short = 's', long = "search")]
    search_query: String,

    #[arg(short = 'm', long = "mono-colors")]
    mono: bool,

    #[arg(short = 'v', long = "verbose")]
    verbose: bool,
}

fn main() -> Result<(), std::io::Error> {
    let total_start = Instant::now();
    let args = Args::parse();
    let mut use_colors = false;
    // Проверяем, поддерживает ли окружение цвета (is_terminal вернет false при перенаправлении в файл)
    if !args.mono {
        use_colors = stdout().is_terminal();
    }
    let mask = args.file;

    //  Если вы захотите искать файлы *.json.gz не только в папке temp, но и во всех её подпапках, вам достаточно просто изменить строку на r"C:\temp\**\*.json.gz". Две звездочки ** включают глубокое сканирование.
    let found_files = find_files_by_pattern(&mask);
    if found_files.is_empty() {
        println!("Файлы не найдены.");
    } else {
        println!("Найдено файлов: {}", found_files.len());
        for path in found_files {
            let start = Instant::now();
            println!("Загрузка и распаковка {}", path.display());
            let tar_gz = File::open(&path)?;
            let mut tar = GzDecoder::new(tar_gz);
            let mut buf = String::new();
            tar.read_to_string(&mut buf)?;
            let mut out: Value = serde_json::from_str(&buf)?;

            clean_json_value(&mut out); // Очистка JSON для корректного отображения и поиска

            let elapsed = start.elapsed();
            println!("{:.5} сек.", elapsed.as_secs_f64());

            let start = Instant::now();
            println!("Результаты поиска для: \"{}\"\n", args.search_query);

            // 3. Запуск поиска от корня ("$")
            let mut filename: &str = "";
            if let Some(filename_str) = path.file_name().and_then(|os_str| os_str.to_str()) {
                filename = filename_str;
            } else {
                println!("Имя файла содержит некорректные UTF-8 символы!");
            };
            search_in_json(
                &out,
                &args.search_query,
                "$",
                use_colors,
                args.verbose,
                filename,
            );
            // println!("{}", serde_json::to_string_pretty(&out)?);
            // let mut archive = Archive::new(tar);
            // archive.unpack(".")?;
            let elapsed = start.elapsed();
            println!(
                "--- Поиск в файле {filename} занял {:.5} сек.",
                elapsed.as_secs_f64()
            );
            drop(buf);
            drop(tar);
        }
    }

    let total_elapsed = total_start.elapsed();
    println!(
        "--- ОБЩИЙ Поиск занял {:.3} сек.",
        total_elapsed.as_secs_f64()
    );

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

fn search_in_json(
    value: &Value,
    query: &str,
    current_path: &str,
    use_colors: bool,
    verbose: bool,
    filename: &str,
) {
    if query.is_empty() {
        return;
    }
    let query_lowercase = query.to_lowercase();

    match value {
        Value::Object(obj) => {
            for (key, val) in obj {
                let next_path = format!("{}.{}", current_path, key);

                // А. Проверка КЛЮЧА
                if key.to_lowercase().contains(&query_lowercase) {
                    let highlighted_key = highlight_match(key, &query_lowercase, use_colors);
                    println!("[Найдено в КЛЮЧЕ файла {}]", filename);
                    // Подсвечиваем совпадение в самом пути
                    println!("Путь: {}.{}", current_path, highlighted_key);
                    if verbose {
                        println!("Значение по этому ключу: {}\n", truncate_value(val))
                    };
                }

                search_in_json(val, query, &next_path, use_colors, verbose, filename);
            }
        }
        Value::Array(arr) => {
            for (index, item) in arr.iter().enumerate() {
                let next_path = format!("{}[{}]", current_path, index);
                search_in_json(item, query, &next_path, use_colors, verbose, filename);
            }
        }
        Value::String(s) => {
            // Б. Проверка ЗНАЧЕНИЯ
            if s.to_lowercase().contains(&query_lowercase) {
                let highlighted_text = highlight_match(s, &query_lowercase, use_colors);
                println!("[Найдено в ЗНАЧЕНИИ файла {}]", filename);
                println!("Путь: {}", current_path);
                if verbose {
                    println!("Текст: \"{}\"\n", highlighted_text);
                }
            }
        }
        _ => {}
    }
}

/// Функция для инвертирования цвета подстроки с сохранением оригинального регистра букв
fn highlight_match(original: &str, query_lowercase: &str, use_colors: bool) -> String {
    if !use_colors {
        return original.to_string();
    }

    let mut result = String::new();
    let original_lowercase = original.to_lowercase();
    let mut current_idx = 0;

    // Ищем все вхождения подстроки
    while let Some(match_start_byte) = original_lowercase[current_idx..].find(query_lowercase) {
        let absolute_start = current_idx + match_start_byte;
        let absolute_end = absolute_start + query_lowercase.len();

        // Добавляем текст ДО совпадения
        result.push_str(&original[current_idx..absolute_start]);

        // Добавляем совпадение с ANSI-инверсией (\x1b[7m — инверсия, \x1b[0m — сброс)
        result.push_str("\x1b[7m");
        result.push_str(&original[absolute_start..absolute_end]);
        result.push_str("\x1b[0m");

        current_idx = absolute_end;
    }

    // Добавляем оставшуюся часть строки
    result.push_str(&original[current_idx..]);
    result
}

fn truncate_value(val: &Value) -> String {
    let s = serde_json::to_string(val).unwrap_or_default();
    if s.len() > 150 {
        format!("{}... [обрезано]", &s[..150])
    } else {
        s
    }
}

fn find_files_by_pattern(pattern: &str) -> Vec<PathBuf> {
    let mut found_files = Vec::new();

    // Функция glob сама разберет "C:\temp\*.json.gz" на каталог и маску.
    // Она также поддерживает рекурсивный поиск, если указать "**/*.json.gz".
    match glob(pattern) {
        Ok(paths) => {
            for entry in paths {
                match entry {
                    Ok(path) => {
                        // Проверяем, что это файл, а не папка
                        if path.is_file() {
                            found_files.push(path);
                        }
                    }
                    // Игнорируем ошибки чтения конкретных файлов (например, проблемы с правами доступа)
                    Err(_) => continue,
                }
            }
        }
        Err(e) => {
            println!("Ошибка: Некорректный синтаксис шаблона поиска: {}", e);
        }
    }

    found_files
}
