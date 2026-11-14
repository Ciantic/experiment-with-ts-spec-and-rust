//! Runs one TimescaleDB container for all test processes and one schema per test.

use std::ffi::c_int;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::net::TcpListener;
use std::os::fd::AsRawFd;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use sqlx::postgres::{PgConnectOptions, PgConnection, PgPoolOptions};
use sqlx::{Connection, PgPool};

const IMAGE: &str = "docker.io/timescale/timescaledb:latest-pg18";
const USER: &str = "postgres";
const PASSWORD: &str = "postgres";
const DATABASE: &str = "postgres";
const POOL_SIZE: u32 = 5;

const STARTUP_TIMEOUT: Duration = Duration::from_secs(120);
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const CONNECT_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(1);
const RETRY_INTERVAL: Duration = Duration::from_millis(100);

static STARTING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static STARTED: OnceLock<PgtestServer> = OnceLock::new();
static REGISTRY: OnceLock<Registry> = OnceLock::new();

/// Handle to the container shared by every test process of the user.
#[derive(Clone)]
pub struct PgtestServer {
    port: u16,
}

impl PgtestServer {
    /// Attaches to the shared container, starting it if no process uses one yet.
    pub async fn start() -> Self {
        let _starting = STARTING.lock().await;
        if let Some(server) = STARTED.get() {
            return server.clone();
        }

        let server = Self::attach().await;
        STARTED.set(server.clone()).ok();
        server
    }

    /// The local port the PostgreSQL wire protocol is served on.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Creates a fresh schema and a pool whose connections default to it.
    pub async fn database(&self) -> PgPool {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        static STARTED_AT: OnceLock<u128> = OnceLock::new();

        let started_at = STARTED_AT.get_or_init(|| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("the system clock is before the epoch")
                .as_nanos()
        });
        let schema = format!(
            "test_{}_{started_at}_{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        );

        let pool = PgPoolOptions::new()
            .max_connections(POOL_SIZE)
            .min_connections(1)
            .connect_with(options(self.port).options([("search_path", format!("{schema},public"))]))
            .await
            .expect("failed to open a pool on the PostgreSQL container");

        sqlx::raw_sql(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&pool)
            .await
            .expect("failed to create the test schema");

        pool
    }

    async fn attach() -> Self {
        let registry = REGISTRY.get_or_init(Registry::open);
        let _slot = registry.lock();
        registry.prune();
        registry.register();

        if let Some(info) = registry.info().filter(|info| info.image == IMAGE)
            && let Some(connection) = probe(info.port).await
        {
            drop(connection);
            return Self { port: info.port };
        }

        if let Some(info) = registry.info() {
            remove_container(&info.container);
        }

        let port = free_port();
        let container = container_name(port);
        start_container(&container, port);

        let server = Self { port };
        let mut connection = server.connect().await;
        sqlx::raw_sql("CREATE EXTENSION IF NOT EXISTS timescaledb")
            .execute(&mut connection)
            .await
            .expect("failed to create the timescaledb extension");

        registry
            .write_info(&ServerInfo {
                image: IMAGE.to_owned(),
                container,
                port,
            })
            .expect("failed to record the shared PostgreSQL container");

        server
    }

    async fn connect(&self) -> PgConnection {
        let deadline = Instant::now() + STARTUP_TIMEOUT;

        loop {
            let attempt = tokio::time::timeout(
                CONNECT_ATTEMPT_TIMEOUT,
                PgConnection::connect_with(&options(self.port)),
            )
            .await;

            let error = match attempt {
                Ok(Ok(connection)) => return connection,
                Ok(Err(error)) => error.to_string(),
                Err(_) => format!("connect attempt timed out after {CONNECT_ATTEMPT_TIMEOUT:?}"),
            };

            if Instant::now() >= deadline {
                panic!("timed out waiting for the PostgreSQL container: {error}");
            }

            tokio::time::sleep(RETRY_INTERVAL).await;
        }
    }
}

fn options(port: u16) -> PgConnectOptions {
    PgConnectOptions::new()
        .host("127.0.0.1")
        .port(port)
        .username(USER)
        .password(PASSWORD)
        .database(DATABASE)
}

/// Tells whether another process already serves the container on `port`.
async fn probe(port: u16) -> Option<PgConnection> {
    tokio::time::timeout(PROBE_TIMEOUT, PgConnection::connect_with(&options(port)))
        .await
        .ok()?
        .ok()
}

/// Where the container listens and who uses it, shared through files.
struct Registry {
    dir: PathBuf,
}

struct ServerInfo {
    image: String,
    container: String,
    port: u16,
}

impl Registry {
    fn open() -> Self {
        let dir = std::env::temp_dir().join(format!("suppilo-pgtest-{}", effective_uid()));
        fs::create_dir_all(dir.join("pids")).expect("failed to create the shared test server dir");
        // SAFETY: the handler only locks a file and runs `podman rm`.
        unsafe { libc::atexit(cleanup) };

        Self { dir }
    }

    /// Serializes startup and cleanup across processes.
    fn lock(&self) -> Lock {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.dir.join("lock"))
            .expect("failed to open the shared test server lock");

        loop {
            // SAFETY: `file` owns the locked descriptor for the guard's lifetime.
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } == 0 {
                return Lock { file };
            }

            let error = io::Error::last_os_error();
            assert_eq!(
                error.kind(),
                io::ErrorKind::Interrupted,
                "failed to lock the shared test server: {error}"
            );
        }
    }

    fn info(&self) -> Option<ServerInfo> {
        let text = fs::read_to_string(self.dir.join("server")).ok()?;
        let mut lines = text.lines();
        let image = lines.next()?.to_owned();
        let container = lines.next()?.to_owned();
        let port = lines.next()?.parse().ok()?;

        Some(ServerInfo {
            image,
            container,
            port,
        })
    }

    fn write_info(&self, info: &ServerInfo) -> io::Result<()> {
        let text = format!("{}\n{}\n{}\n", info.image, info.container, info.port);
        fs::write(self.dir.join("server"), text)
    }

    fn register(&self) {
        let _ = fs::write(self.pid_file(), []);
    }

    fn unregister(&self) {
        let _ = fs::remove_file(self.pid_file());
    }

    /// Forgets the users that died without running their cleanup.
    fn prune(&self) {
        let Ok(entries) = fs::read_dir(self.dir.join("pids")) else {
            return;
        };

        for entry in entries.flatten() {
            let Some(pid) = entry
                .file_name()
                .to_str()
                .and_then(|name| name.parse().ok())
            else {
                continue;
            };

            if !alive(pid) {
                let _ = fs::remove_file(entry.path());
            }
        }
    }

    fn has_users(&self) -> bool {
        fs::read_dir(self.dir.join("pids")).is_ok_and(|mut entries| entries.next().is_some())
    }

    fn pid_file(&self) -> PathBuf {
        self.dir.join("pids").join(std::process::id().to_string())
    }
}

struct Lock {
    file: File,
}

impl Drop for Lock {
    fn drop(&mut self) {
        // SAFETY: the descriptor is still open and was locked by this process.
        unsafe { libc::flock(self.file.as_raw_fd(), libc::LOCK_UN) };
    }
}

/// Runs when a test process exits, so the last one out removes the container.
extern "C" fn cleanup() {
    let Some(registry) = REGISTRY.get() else {
        return;
    };

    let _slot = registry.lock();
    registry.unregister();
    registry.prune();

    if !registry.has_users()
        && let Some(info) = registry.info()
    {
        remove_container(&info.container);
        let _ = fs::remove_file(registry.dir.join("server"));
    }
}

fn alive(pid: c_int) -> bool {
    // SAFETY: signal 0 has no effect beyond checking that the pid exists.
    if unsafe { libc::kill(pid, 0) } == 0 {
        return true;
    }

    // A live process of another user answers with EPERM instead of ESRCH.
    io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

fn effective_uid() -> u32 {
    // SAFETY: `geteuid` has no preconditions.
    unsafe { libc::geteuid() }
}

fn container_name(port: u16) -> String {
    format!("suppilo-pgtest-{}-{port}", effective_uid())
}

fn start_container(container: &str, port: u16) {
    let output = Command::new("podman")
        .args([
            "run",
            "--rm",
            "--detach",
            "--name",
            container,
            // Keeps the data directory in RAM; initdb is what dominates startup.
            // Applies to whatever path the image itself declares as a volume.
            "--image-volume",
            "tmpfs",
            "--publish",
            &format!("127.0.0.1:{port}:5432"),
            "--env",
            "POSTGRES_USER=postgres",
            "--env",
            "POSTGRES_PASSWORD=postgres",
            "--env",
            "POSTGRES_DB=postgres",
            IMAGE,
        ])
        .output()
        .unwrap_or_else(|error| {
            panic!(
                "failed to start `podman`: {error}. Install Podman from https://podman.io first."
            )
        });

    assert!(
        output.status.success(),
        "`podman run` failed: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
}

fn remove_container(container: &str) {
    let _ = Command::new("podman")
        .args(["rm", "--force", "--time", "0", container])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

fn free_port() -> u16 {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("failed to reserve a local port");
    listener
        .local_addr()
        .expect("failed to read the reserved port")
        .port()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn serves_queries() {
        let db = PgtestServer::start().await.database().await;

        let (one,): (i32,) = sqlx::query_as("SELECT 1").fetch_one(&db).await.unwrap();
        assert_eq!(one, 1);

        sqlx::query("CREATE TABLE widget (name TEXT NOT NULL)")
            .execute(&db)
            .await
            .unwrap();

        let (name,): (String,) =
            sqlx::query_as("INSERT INTO widget (name) VALUES ($1) RETURNING name")
                .bind("Thingie")
                .fetch_one(&db)
                .await
                .unwrap();
        assert_eq!(name, "Thingie");
    }

    #[tokio::test]
    async fn reuses_one_container() {
        let first = PgtestServer::start().await;
        let second = PgtestServer::start().await;

        assert_eq!(first.port(), second.port());
    }

    #[tokio::test]
    async fn databases_are_isolated() {
        let server = PgtestServer::start().await;
        let first = server.database().await;
        let second = server.database().await;

        sqlx::query("CREATE TABLE widget (name TEXT NOT NULL)")
            .execute(&first)
            .await
            .unwrap();

        let table: Option<String> = sqlx::query_scalar("SELECT to_regclass('widget')::text")
            .fetch_one(&second)
            .await
            .unwrap();
        assert!(table.is_none());
    }

    #[tokio::test]
    async fn has_timescaledb() {
        let db = PgtestServer::start().await.database().await;

        sqlx::query("CREATE TABLE reading (taken_at TIMESTAMPTZ NOT NULL)")
            .execute(&db)
            .await
            .unwrap();
        sqlx::query("SELECT create_hypertable('reading', 'taken_at')")
            .execute(&db)
            .await
            .unwrap();
    }
}
