extern crate flate2;

use clap::Parser;
use flate2::read::GzDecoder;
use glob::glob;
use indicatif::{ProgressBar, ProgressStyle};
use serde_json;
use serde_json::Value;
use std::fs::File;
use std::io::{self, IsTerminal, Read, Seek, Write, stdout};
use std::path::PathBuf;
use std::time::Instant;

#[derive(Parser, Debug, Clone)]
#[command(name = "search")]
struct Args {
    /// Filepath with mask(if nedded). Supports recursion (ex. "**/*.json.gz")
    #[arg(short = 'f', long = "file")]
    file: String,

    /// Search string in files
    #[arg(short = 's', long = "search")]
    search_query: String,

    /// Do not use ansi inversions
    #[arg(short = 'm', long = "mono", default_value_t = if stdout().is_terminal() { false } else { true })]
    mono: bool,

    /// Print json values where serch string was found
    #[arg(short = 'v', long = "verbose", default_value_t = false)]
    verbose: bool,

    /// Quiet fast mode (only progress and files where found)
    #[arg(short = 'q', long = "quiet", default_value_t = false)]
    quiet: bool,
}

// Обертка над Read-потоком для подсчета прочитанных байт
struct ProgressReader<'a, R> {
    inner: R,
    total: u64,
    current: u64,
    filename: &'a str,
    pb: ProgressBar,
}

impl<'a, R: Read + Seek> ProgressReader<'a, R> {
    fn new(mut inner: R, total: u64, filename: &'a str) -> io::Result<Self> {
        let pb = ProgressBar::new(total);

        pb.set_style(
            ProgressStyle::default_bar()
                .template(
                    "{msg} {spinner:.green} [{bar:20.cyan/blue}] ({percent}%) {elapsed_hhmmss}",
                )
                .unwrap(),
        );

        pb.set_message(format!("Loading {}", filename));

        Ok(Self {
            inner,
            total,
            filename,
            current: 0,
            pb,
        })
    }
}

impl<'a, R: Read + Seek> Read for ProgressReader<'a, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        if n > 0 {
            self.current += n as u64;
            // Обновляем прогресс-бар
            std::io::stdout().flush().unwrap();
            self.pb.set_position(self.current);
            std::io::stdout().flush().unwrap();
        }
        Ok(n)
    }
}

impl<'a, R> Drop for ProgressReader<'a, R> {
    fn drop(&mut self) {
        // Принудительно завершаем прогресс-бар, если он еще не завершен
        self.pb
            .finish_with_message(format!("Loaded {}", self.filename));
        self.pb.finish();
    }
}

fn main() -> Result<(), std::io::Error> {
    let total_start = Instant::now();
    let args = Args::parse();
    let mask = &args.file;

    //  Если вы захотите искать файлы *.json.gz не только в папке temp, но и во всех её подпапках, вам достаточно просто изменить строку на r"C:\temp\**\*.json.gz". Две звездочки ** включают глубокое сканирование.
    let (found_files, total) = find_files_by_pattern(&mask);
    if found_files.is_empty() {
        eprintln!("Файлы не найдены.");
    } else {
        if !args.quiet {
            println!("Найдено файлов: {}", found_files.len())
        };
        // let pbm = ProgressBar::new(total as u64);
        // pbm.set_style(
        //     ProgressStyle::default_bar()
        //         .template(
        //             "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({eta})",
        //         )
        //         .unwrap()
        //         .progress_chars("=>-"),
        // );

        for path in found_files {
            let start = Instant::now();
            let mut size = 0;
            if !args.quiet {
                println!("\nЗагрузка и распаковка {}", path.display());
            };
            // Пытаемся получить метаданные и размер
            let json_gz = File::open(&path)?;
            match json_gz.metadata() {
                Ok(metadata) => {
                    size = metadata.len();
                    // println!("Найден файл: {:?}, размер: {} байт", path, size);
                }
                Err(e) => {
                    // Файл существовал при сканировании glob, но сейчас недоступен
                    eprintln!("Не удалось получить размер для {:?}: {}", path, e);
                }
            }

            let mut filename: &str = "";
            if let Some(filename_str) = path.file_name().and_then(|os_str| os_str.to_str()) {
                filename = filename_str;
            } else {
                eprintln!(
                    "Имя файла содержит некорректные UTF-8 символы и не может быть отображено!"
                );
            };
            let mut out: Value;
            let unpacked_size: usize;
            {
                // Для корректной работы progress-bar создаём отдельную область видимости, по выходу из которой всё корректно завершается, а не "висит" до тех пор, пока не будет обработано.

                let mut buf = String::new();
                let progress_reader = ProgressReader::new(json_gz, size, filename)?;
                let mut tar = GzDecoder::new(progress_reader);

                tar.read_to_string(&mut buf)?;
                std::io::stdout().flush().unwrap();

                out = serde_json::from_str(&buf)?;
                unpacked_size = buf.len();
            }
            let pbs = ProgressBar::new_spinner();
            pbs.set_message("Search");
            pbs.set_style(
                ProgressStyle::default_spinner()
                    .tick_strings(&["-", "\\", "|", "/", " "])
                    .template("{msg} {spinner:.green}")
                    .unwrap(),
            );

            clean_json_value(&mut out, &pbs); // Очистка JSON для корректного отображения и поиска
            // pbm.inc(size);
            let elapsed = start.elapsed();
            if !args.quiet {
                println!(
                    "Загрузка и подготовака файла размером {} заняла {:.5} сек.",
                    human_readable_size(unpacked_size as u64),
                    elapsed.as_secs_f64()
                );
            }

            let start = Instant::now();
            if !args.quiet {
                println!("Ищем в {} фразу: \"{}\"", filename, args.search_query);
            }
            // 3. Запуск поиска от корня ("$")
            search_in_json(&out, "$", &args, filename, &pbs);
            // println!("{}", serde_json::to_string_pretty(&out)?);
            // let mut archive = Archive::new(tar);
            // archive.unpack(".")?;
            pbs.finish_with_message("Finished");
            pbs.finish();
            let elapsed = start.elapsed();
            if !args.quiet {
                println!(
                    "--- Поиск в файле {filename} занял {:.5} сек.",
                    elapsed.as_secs_f64()
                );
            }
        }
    }

    let total_elapsed = total_start.elapsed();
    println!(
        "--- ОБЩИЙ Поиск занял {:.3} сек.",
        total_elapsed.as_secs_f64()
    );

    Ok(())
}

fn clean_json_value(value: &mut Value, pb: &ProgressBar) {
    pb.tick();
    match value {
        // Если это строка, пробуем распарсить её как внутренний JSON
        Value::String(s) => {
            // Если строка валидна как JSON (например, содержит внутренний объект или массив)
            if let Ok(inner_value) = serde_json::from_str::<Value>(s) {
                let mut decoded_inner = inner_value;
                // Рекурсивно чистим то, что было внутри этой строки
                clean_json_value(&mut decoded_inner, pb);
                *value = decoded_inner;
            }
        }
        // Если это массив, чистим каждый элемент
        Value::Array(arr) => {
            for item in arr {
                clean_json_value(item, pb);
            }
        }
        // Если это объект, чистим каждое значение по ключу
        Value::Object(obj) => {
            for (_key, val) in obj.iter_mut() {
                clean_json_value(val, pb);
            }
        }
        _ => {}
    }
}

fn search_in_json(
    value: &Value,
    current_path: &str,
    args: &Args,
    filename: &str,
    pb: &ProgressBar,
) -> bool {
    if args.search_query.is_empty() {
        return false;
    }
    let query_lowercase = args.search_query.to_lowercase();
    pb.tick();
    match value {
        Value::Object(obj) => {
            for (key, val) in obj {
                let next_path = format!("{}.{}", current_path, key);

                // А. Проверка КЛЮЧА
                if key.to_lowercase().contains(&query_lowercase) {
                    let highlighted_key = highlight_match(key, &query_lowercase, args.mono);
                    println!("[Найдено в КЛЮЧЕ файла {}]", filename);
                    // Подсвечиваем совпадение в самом пути
                    if args.quiet {
                        return true;
                    } else {
                        println!("Путь: {}.{}", current_path, highlighted_key);
                    }
                    if args.verbose {
                        println!("Значение по этому ключу: {}\n", truncate_value(val))
                    };
                }

                if search_in_json(val, &next_path, args, filename, pb) {
                    return true;
                }
            }
        }
        Value::Array(arr) => {
            for (index, item) in arr.iter().enumerate() {
                let next_path = format!("{}[{}]", current_path, index);
                if search_in_json(item, &next_path, args, filename, pb) {
                    return true;
                }
            }
        }
        Value::String(s) => {
            // Б. Проверка ЗНАЧЕНИЯ
            if s.to_lowercase().contains(&query_lowercase) {
                let highlighted_text = highlight_match(s, &query_lowercase, args.mono);
                println!("[Найдено в ЗНАЧЕНИИ файла {}]", filename);
                if !args.quiet {
                    println!("Путь: {}", current_path);
                } else {
                    return true;
                }
                if args.verbose {
                    println!("Текст: \"{}\"\n", highlighted_text);
                }
            }
        }
        _ => {}
    }
    false
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

fn find_files_by_pattern(pattern: &str) -> (Vec<PathBuf>, u64) {
    let mut found_files = Vec::new();
    let mut total: u64 = 0;
    // Функция glob сама разберет "C:\temp\*.json.gz" на каталог и маску.
    // Она также поддерживает рекурсивный поиск, если указать "**/*.json.gz".
    match glob(pattern) {
        Ok(paths) => {
            for entry in paths {
                match entry {
                    Ok(path) => {
                        // Проверяем, что это файл, а не папка
                        let metadata = path.metadata().expect("file read error!");
                        if metadata.is_file() {
                            found_files.push(path);
                            total += metadata.len();
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
    (found_files, total)
}

fn human_readable_size(bytes: u64) -> String {
    if bytes == 0 {
        return String::from("0b");
    }

    let mut size = bytes as f64;
    let units = ["b", "Kb", "Mb", "Gb", "Tb"];
    let mut unit_index = 0;

    // Пока размер больше 1024 и у нас есть старшие единицы
    while size >= 1024.0 && unit_index < units.len() - 1 {
        size /= 1024.0;
        unit_index += 1;
    }

    // Форматируем число, округляя до десятой доли
    format!("{:.1}{}", size, units[unit_index])
}
