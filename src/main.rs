use lamco_data_control::{Content, DataControl};
use rusqlite::{params, Connection, OptionalExtension};
use std::error::Error;
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const PNG_SIGNATURE: [u8; 8] = [
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A,
];


fn create_thumbnail(id: i64, bytes: &[u8]) -> Result<String, Box<dyn Error>> {
    let cache_dir = match std::env::var_os("XDG_CACHE_HOME") {
        Some(path) => PathBuf::from(path).join("mokka-clipboard/thumbnails"),
        None => PathBuf::from(std::env::var("HOME")?)
            .join(".cache/mokka-clipboard/thumbnails"),
    };

    std::fs::create_dir_all(&cache_dir)?;

    let thumbnail_path = cache_dir.join(format!("{id}.png"));

    if thumbnail_path.exists() {
        return Ok(thumbnail_path.to_string_lossy().into_owned());
    }

    let original = image::load_from_memory_with_format(
        bytes,
        image::ImageFormat::Png,
    )?;

    let thumbnail = original.thumbnail(320, 240);
    thumbnail.save(&thumbnail_path)?;

    Ok(thumbnail_path.to_string_lossy().into_owned())
}




struct NewEntry {
    kind: String,
    mime_type: String,
    content: Vec<u8>,
    preview: String,
}

enum Event {
    ClipboardChanged(Vec<String>),
    Command(String),
}

struct Database {
    connection: Connection,
}

impl Database {
    fn open() -> Result<Self, Box<dyn Error>> {
        let data_dir = match std::env::var_os("XDG_DATA_HOME") {
            Some(path) => PathBuf::from(path).join("mokka-clipboard"),
            None => PathBuf::from(std::env::var("HOME")?)
                .join(".local/share/mokka-clipboard"),
        };

        std::fs::create_dir_all(&data_dir)?;

        let db_path = data_dir.join("mokka.db");
        let connection = Connection::open(&db_path)?;

        connection.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS entries (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                kind TEXT NOT NULL CHECK(kind IN ('text', 'image')),
                mime_type TEXT NOT NULL,
                content BLOB NOT NULL,
                preview TEXT,
                created_at TEXT NOT NULL
                    DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
                is_favorite INTEGER NOT NULL DEFAULT 0
                    CHECK(is_favorite IN (0, 1))
            );

            CREATE INDEX IF NOT EXISTS idx_entries_created_at
            ON entries(created_at DESC);
            ",
        )?;


        Ok(Self { connection })
    }

    fn insert(&self, entry: &NewEntry) -> rusqlite::Result<i64> {
        self.connection.execute(
            "INSERT INTO entries (kind, mime_type, content, preview)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                entry.kind,
                entry.mime_type,
                entry.content,
                entry.preview
            ],
        )?;

        Ok(self.connection.last_insert_rowid())
    }

    fn get_entry(&self, id: i64) -> rusqlite::Result<Option<(String, Vec<u8>)>> {
        self.connection
            .query_row(
                "SELECT mime_type, content FROM entries WHERE id = ?1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
    }

    fn list_ids(&self) -> rusqlite::Result<Vec<i64>> {
        let mut statement =
            self.connection.prepare("SELECT id FROM entries ORDER BY id ASC")?;

        let rows = statement.query_map([], |row| row.get(0))?;

        let mut ids = Vec::new();
        for row in rows {
            ids.push(row?);
        }

        Ok(ids)
    }

    fn count(&self) -> rusqlite::Result<i64> {
        self.connection
            .query_row("SELECT COUNT(*) FROM entries", [], |row| row.get(0))
    }

    fn entry_json(
        &self,
        id: i64,
        kind: String,
        mime_type: String,
        preview: String,
        created_at: String,
        is_favorite: bool,
    ) -> Result<serde_json::Value, Box<dyn Error>> {
        let thumbnail_path = if kind == "image" {
            match self.get_entry(id)? {
                Some((_mime, content)) => Some(create_thumbnail(id, &content)?),
                None => None,
            }
        } else {
            None
        };

        Ok(serde_json::json!({
            "id": id,
            "kind": kind,
            "mime_type": mime_type,
            "preview": preview,
            "created_at": created_at,
            "is_favorite": is_favorite,
            "thumbnail_path": thumbnail_path
        }))
    }

    fn list_json(&self) -> Result<(), Box<dyn Error>> {
        let mut statement = self.connection.prepare(
            "SELECT id, kind, mime_type, preview, created_at, is_favorite
             FROM entries ORDER BY id DESC LIMIT 200",
        )?;

        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, i64>(5)? == 1,
            ))
        })?;

        let mut records = Vec::new();
        for row in rows {
            records.push(row?);
        }
        drop(statement);

        let mut entries = Vec::new();
        for (id, kind, mime_type, preview, created_at, is_favorite) in records {
            entries.push(self.entry_json(
                id, kind, mime_type, preview, created_at, is_favorite
            )?);
        }

        println!("{}", serde_json::to_string(&entries)?);
        Ok(())
    }

    fn search_json(&self, term: &str) -> Result<(), Box<dyn Error>> {
        let mut statement = self.connection.prepare(
            "SELECT id, kind, mime_type, preview, created_at, is_favorite
             FROM entries
             WHERE instr(lower(COALESCE(preview, '')), lower(?1)) > 0
             ORDER BY id DESC LIMIT 200",
        )?;

        let rows = statement.query_map([term], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, i64>(5)? == 1,
            ))
        })?;

        let mut records = Vec::new();
        for row in rows {
            records.push(row?);
        }
        drop(statement);

        let mut entries = Vec::new();
        for (id, kind, mime_type, preview, created_at, is_favorite) in records {
            entries.push(self.entry_json(
                id, kind, mime_type, preview, created_at, is_favorite
            )?);
        }

        println!("{}", serde_json::to_string(&entries)?);
        Ok(())
    }

    fn favorites_json(&self) -> Result<(), Box<dyn Error>> {
        let mut statement = self.connection.prepare(
            "SELECT id, kind, mime_type, preview, created_at, is_favorite
             FROM entries
             WHERE is_favorite = 1
             ORDER BY id DESC LIMIT 200",
        )?;

        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, i64>(5)? == 1,
            ))
        })?;

        let mut records = Vec::new();
        for row in rows {
            records.push(row?);
        }
        drop(statement);

        let mut entries = Vec::new();
        for (id, kind, mime_type, preview, created_at, is_favorite) in records {
            entries.push(self.entry_json(
                id, kind, mime_type, preview, created_at, is_favorite
            )?);
        }

        println!("{}", serde_json::to_string(&entries)?);
        Ok(())
    }

    fn set_favorite(&self, id: i64, favorite: bool) -> rusqlite::Result<bool> {
        let changed = self.connection.execute(
            "UPDATE entries SET is_favorite = ?1 WHERE id = ?2",
            params![favorite as i64, id],
        )?;

        Ok(changed > 0)
    }

    fn list_recent(&self, limit: i64) -> rusqlite::Result<()> {
        let mut statement = self.connection.prepare(
            "SELECT id, kind, mime_type, length(content), created_at, is_favorite
             FROM entries ORDER BY id DESC LIMIT ?1",
        )?;

        let entries = statement.query_map([limit], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, i64>(5)?,
            ))
        })?;

        println!("\n===== ÚLTIMAS ENTRADAS DO HISTÓRICO =====");

        let mut found = false;

        for entry in entries {
            found = true;
            let (id, kind, mime, size, created_at, favorite) = entry?;
            let star = if favorite == 1 { " ★" } else { "" };

            println!(
                "#{id} | {kind} | {mime} | {size} bytes | {created_at}{star}"
            );
        }

        if !found {
            println!("O histórico ainda está vazio.");
        }

        println!("========================================\n");
        Ok(())
    }
}

fn read_text(clipboard: &DataControl, mime_types: &[String]) -> Option<NewEntry> {
    let preferred_types = [
        "text/plain;charset=utf-8",
        "text/plain",
        "UTF8_STRING",
        "STRING",
    ];

    for mime in preferred_types {
        let Some(actual_mime) = mime_types
            .iter()
            .find(|available| available.eq_ignore_ascii_case(mime))
        else {
            continue;
        };

        match clipboard.read(actual_mime) {
            Ok(Some(bytes)) => {
                let preview = String::from_utf8_lossy(&bytes).into_owned();

                return Some(NewEntry {
                    kind: "text".to_string(),
                    mime_type: actual_mime.clone(),
                    content: bytes,
                    preview,
                });
            }
            Ok(None) => {}
            Err(error) => eprintln!("Erro ao ler texto: {error}"),
        }
    }

    None
}

fn read_image(clipboard: &DataControl, mime_types: &[String]) -> Option<NewEntry> {
    if !mime_types.iter().any(|mime| mime == "image/png") {
        eprintln!("Imagem detectada, mas o formato ainda não é suportado.");
        return None;
    }

    match clipboard.read("image/png") {
        Ok(Some(bytes)) => {
            if !bytes.starts_with(&PNG_SIGNATURE) {
                eprintln!("Imagem PNG inválida. Não será armazenada.");
                return None;
            }

            Some(NewEntry {
                kind: "image".to_string(),
                mime_type: "image/png".to_string(),
                content: bytes,
                preview: "Imagem PNG".to_string(),
            })
        }
        Ok(None) => {
            eprintln!("Não foi possível obter os dados da imagem.");
            None
        }
        Err(error) => {
            eprintln!("Erro ao ler imagem PNG: {error}");
            None
        }
    }
}

fn capture_selection(
    clipboard: &DataControl,
    mime_types: &[String],
) -> Option<NewEntry> {
    if mime_types.is_empty() {
        println!("[CLIPBOARD LIMPO] Histórico preservado.");
        return None;
    }

    if mime_types.iter().any(|mime| mime.starts_with("image/")) {
        read_image(clipboard, mime_types)
    } else {
        read_text(clipboard, mime_types)
    }
}

fn get_socket_path() -> Result<PathBuf, Box<dyn Error>> {
    if let Some(runtime_dir) = std::env::var_os("XDG_RUNTIME_DIR") {
        Ok(PathBuf::from(runtime_dir).join("mokka-clipboard.sock"))
    } else {
        let dir = PathBuf::from(std::env::var("HOME")?)
            .join(".cache/mokka-clipboard");
        std::fs::create_dir_all(&dir)?;
        Ok(dir.join("mokka.sock"))
    }
}

fn get_event_socket_path() -> Result<PathBuf, Box<dyn Error>> {
    if let Some(runtime_dir) = std::env::var_os("XDG_RUNTIME_DIR") {
        Ok(PathBuf::from(runtime_dir).join("mokka-clipboard-events.sock"))
    } else {
        let dir = PathBuf::from(std::env::var("HOME")?)
            .join(".cache/mokka-clipboard");
        std::fs::create_dir_all(&dir)?;
        Ok(dir.join("mokka-events.sock"))
    }
}

fn start_event_server(
    path: PathBuf,
) -> Result<Arc<Mutex<Vec<UnixStream>>>, Box<dyn Error>> {
    if path.exists() {
        if UnixStream::connect(&path).is_ok() {
            return Err("O servidor de eventos do Mokka já está em execução.".into());
        }

        std::fs::remove_file(&path)?;
    }

    let listener = UnixListener::bind(&path)?;
    let clients = Arc::new(Mutex::new(Vec::<UnixStream>::new()));
    let connected_clients = Arc::clone(&clients);

    println!("Socket de eventos: {}", path.display());

    std::thread::spawn(move || {
        for incoming in listener.incoming() {
            match incoming {
                Ok(stream) => {
                    if let Err(error) =
                        stream.set_write_timeout(Some(Duration::from_millis(100)))
                    {
                        eprintln!("Erro ao configurar cliente de eventos: {error}");
                        continue;
                    }

                    if let Ok(mut clients) = connected_clients.lock() {
                        clients.push(stream);
                    }
                }
                Err(error) => {
                    eprintln!("Erro no socket de eventos: {error}");
                }
            }
        }
    });

    Ok(clients)
}

fn notify_subscribers(clients: &Arc<Mutex<Vec<UnixStream>>>) {
    if let Ok(mut clients) = clients.lock() {
        clients.retain_mut(|stream| stream.write_all(b"refresh\n").is_ok());
    }
}

fn start_ipc(path: PathBuf, sender: Sender<Event>) -> Result<(), Box<dyn Error>> {
    if path.exists() {
        if UnixStream::connect(&path).is_ok() {
            return Err("O Mokka já está em execução.".into());
        }

        std::fs::remove_file(&path)?;
    }

    let listener = UnixListener::bind(&path)?;
    println!("Socket de comandos: {}", path.display());

    std::thread::spawn(move || {
        for incoming in listener.incoming() {
            match incoming {
                Ok(mut stream) => {
                    let mut buffer = [0_u8; 64];

                    match stream.read(&mut buffer) {
                        Ok(size) if size > 0 => {
                            let command =
                                String::from_utf8_lossy(&buffer[..size]).trim().to_string();

                            let _ = sender.send(Event::Command(command));
                        }
                        Ok(_) => {}
                        Err(error) => eprintln!("Erro no comando IPC: {error}"),
                    }
                }
                Err(error) => eprintln!("Erro no socket do Mokka: {error}"),
            }
        }
    });

    Ok(())
}

fn send_command(command: &str) -> Result<(), Box<dyn Error>> {
    let path = get_socket_path()?;
    let mut stream = UnixStream::connect(&path)
        .map_err(|_| "Mokka não está em execução. Inicie-o com --daemon.")?;

    use std::io::Write;
    stream.write_all(command.as_bytes())?;

    Ok(())
}

fn restore_entry(
    database: &Database,
    clipboard: &DataControl,
    ids: &[i64],
    position: usize,
    direction: &str,
) -> Result<(), Box<dyn Error>> {
    let id = ids[position];

    if let Some((mime_type, bytes)) = database.get_entry(id)? {
        let content = Content::new().data(mime_type.clone(), bytes);
        clipboard.set_selection(content)?;

        println!(
            "[{direction}] Entrada #{id} restaurada | posição {} de {} | {mime_type}",
            position + 1,
            ids.len()
        );
    }

    Ok(())
}

fn run_daemon(database: Database) -> Result<(), Box<dyn Error>> {
    println!("========================================");
    println!("       MOKKA CLIPBOARD — DAEMON");
    println!("========================================");

    let mut ids = database.list_ids()?;
    let mut position = if ids.is_empty() {
        None
    } else {
        Some(ids.len() - 1)
    };

    println!("Entradas carregadas: {}", ids.len());

    let clipboard = DataControl::connect()?;
    println!("Conexão Wayland estabelecida.");
    println!("Protocolo: {}", clipboard.protocol());

    let (sender, receiver) = mpsc::channel::<Event>();

    let callback_sender = sender.clone();
    clipboard.on_change(move |mime_types| {
        let _ = callback_sender.send(Event::ClipboardChanged(mime_types));
    });

    start_ipc(get_socket_path()?, sender)?;

    let event_clients = start_event_server(get_event_socket_path()?)?;

    println!("\nMokka ativo. Comandos disponíveis:");
    println!("  --previous  Voltar à entrada anterior");
    println!("  --next      Avançar para a próxima entrada");
    println!("Copie textos e imagens para alimentar o histórico.\n");

    event_loop(
        database,
        clipboard,
        &mut ids,
        &mut position,
        receiver,
        event_clients,
    )
}

fn event_loop(
    database: Database,
    clipboard: DataControl,
    ids: &mut Vec<i64>,
    position: &mut Option<usize>,
    receiver: Receiver<Event>,
    event_clients: Arc<Mutex<Vec<UnixStream>>>,
) -> Result<(), Box<dyn Error>> {
    loop {
        match receiver.recv_timeout(Duration::from_millis(200)) {
            Ok(Event::ClipboardChanged(mime_types)) => {
                if let Some(entry) = capture_selection(&clipboard, &mime_types) {
                    let size = entry.content.len();
                    let kind = entry.kind.clone();

                    match database.insert(&entry) {
                        Ok(id) => {
                            ids.push(id);
                            *position = Some(ids.len() - 1);

                            println!(
                                "[SALVO] #{id} | {kind} | {size} bytes | posição atualizada"
                            );

                            notify_subscribers(&event_clients);
                        }
                        Err(error) => {
                            eprintln!("Erro ao salvar no histórico: {error}");
                        }
                    }
                }
            }

            Ok(Event::Command(command)) => {
                match command.as_str() {
                    "previous" => {
                        match *position {
                            Some(current) if current > 0 => {
                                let target = current - 1;
                                restore_entry(
                                    &database,
                                    &clipboard,
                                    ids,
                                    target,
                                    "ANTERIOR",
                                )?;
                                *position = Some(target);
                            }
                            _ => println!("[ANTERIOR] Não há entrada anterior."),
                        }
                    }

                    "next" => {
                        match *position {
                            Some(current) if current + 1 < ids.len() => {
                                let target = current + 1;
                                restore_entry(
                                    &database,
                                    &clipboard,
                                    ids,
                                    target,
                                    "PRÓXIMA",
                                )?;
                                *position = Some(target);
                            }
                            _ => println!("[PRÓXIMA] Não há próxima entrada."),
                        }
                    }

                    _ if command.starts_with("restore:") => {
                        let id_text = &command["restore:".len()..];

                        match id_text.parse::<i64>() {
                            Ok(id) => {
                                match ids.iter().position(|entry_id| *entry_id == id) {
                                    Some(target) => {
                                        restore_entry(
                                            &database,
                                            &clipboard,
                                            ids,
                                            target,
                                            "RESTAURAR",
                                        )?;
                                        *position = Some(target);
                                    }
                                    None => {
                                        println!(
                                            "[RESTAURAR] Não existe uma entrada com ID {id}."
                                        );
                                    }
                                }
                            }
                            Err(_) => {
                                eprintln!("ID inválido no comando IPC: {command}");
                            }
                        }
                    }

                    _ => eprintln!("Comando desconhecido: {command}"),
                }
            }

            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err("Canal de eventos encerrado.".into());
            }
        }
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().collect();
    let database = Database::open()?;

    if args.len() == 2 {
        match args[1].as_str() {
            "--previous" => return send_command("previous"),
            "--next" => return send_command("next"),
            "--list" => {
                println!("Entradas armazenadas: {}", database.count()?);
                return Ok(database.list_recent(20)?);
            }
            "--list-json" => return database.list_json(),
            "--favorites" => return database.favorites_json(),
            "--daemon" => return run_daemon(database),
            _ => {}
        }
    }


    if args.len() == 3 && args[1] == "--search" {
        return database.search_json(&args[2]);
    }

    if args.len() == 3 && (args[1] == "--favorite" || args[1] == "--unfavorite") {
        let id: i64 = args[2].parse()?;
        let favorite = args[1] == "--favorite";

        if database.set_favorite(id, favorite)? {
            println!(
                "Entrada {id}: {}.",
                if favorite { "adicionada aos favoritos" }
                else { "removida dos favoritos" }
            );
        } else {
            println!("Não existe uma entrada com ID {id}.");
        }

        return Ok(());
    }

    if args.len() == 3 && args[1] == "--restore" {
        let id: i64 = args[2].parse()?;
        return send_command(&format!("restore:{id}"));
    }

    println!("Uso:");
    println!("  mokka-clipboard --daemon");
    println!("  mokka-clipboard --previous");
    println!("  mokka-clipboard --next");
    println!("  mokka-clipboard --list");
    println!("  mokka-clipboard --list-json");
    println!("  mokka-clipboard --search \"TERMO\"");
    println!("  mokka-clipboard --favorites");
    println!("  mokka-clipboard --favorite ID");
    println!("  mokka-clipboard --unfavorite ID");
    println!("  mokka-clipboard --restore ID");

    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("Erro no Mokka Clipboard: {error}");
        std::process::exit(1);
    }
}
