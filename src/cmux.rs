use anyhow::{Context, Result};
use serde::de::{DeserializeOwned, IgnoredAny};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::env;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

const CMUX_SOCKET_PATH_ENV: &str = "CMUX_SOCKET_PATH";

#[derive(Debug, Serialize)]
struct RpcRequest<'a, P> {
    method: &'a str,
    params: &'a P,
}

#[derive(Debug, Deserialize)]
struct RpcResponse<T> {
    ok: bool,
    result: Option<T>,
    error: Option<RpcFailure>,
}

#[derive(Debug, Deserialize)]
struct RpcFailure {
    message: Option<String>,
    #[serde(flatten)]
    extra: serde_json::Map<String, Value>,
}

#[derive(Debug, Deserialize)]
struct CreateWorkspaceResult {
    workspace_id: Option<String>,
}

#[derive(Debug, Serialize)]
struct CreateWorkspaceParams<'a> {
    cwd: &'a str,
}

#[derive(Debug, Serialize)]
struct SelectWorkspaceParams<'a> {
    workspace_id: &'a str,
}

trait RpcMethod {
    type Params<'a>: Serialize
    where
        Self: 'a;
    type Result: DeserializeOwned;

    const METHOD: &'static str;
}

struct WorkspaceCreate;
struct WorkspaceSelect;

impl RpcMethod for WorkspaceCreate {
    type Params<'a> = CreateWorkspaceParams<'a>;
    type Result = CreateWorkspaceResult;

    const METHOD: &'static str = "workspace.create";
}

impl RpcMethod for WorkspaceSelect {
    type Params<'a> = SelectWorkspaceParams<'a>;
    type Result = IgnoredAny;

    const METHOD: &'static str = "workspace.select";
}

#[derive(Debug)]
struct CmuxClient {
    socket_path: PathBuf,
}

pub fn create_workspace(path: &Path) -> Result<String> {
    let client = CmuxClient::from_env()?;
    let path_str = path.to_string_lossy();
    let result = client.call::<WorkspaceCreate>(&CreateWorkspaceParams { cwd: &path_str })?;
    result
        .workspace_id
        .context("workspace.create did not return workspace_id")
}

pub fn select_workspace(workspace_id: &str) -> Result<()> {
    let client = CmuxClient::from_env()?;
    let _: IgnoredAny = client.call::<WorkspaceSelect>(&SelectWorkspaceParams { workspace_id })?;
    Ok(())
}

impl CmuxClient {
    fn from_env() -> Result<Self> {
        let socket_path = env::var_os(CMUX_SOCKET_PATH_ENV)
            .map(PathBuf::from)
            .context("CMUX_SOCKET_PATH is not set")?;
        Ok(Self { socket_path })
    }

    fn call<M>(&self, params: &M::Params<'_>) -> Result<M::Result>
    where
        M: RpcMethod,
    {
        let mut stream = self.connect()?;
        self.write_request(&mut stream, M::METHOD, params)?;
        let raw_response = self.read_response(stream, M::METHOD)?;
        decode_response(M::METHOD, &raw_response)
    }

    fn connect(&self) -> Result<UnixStream> {
        UnixStream::connect(&self.socket_path).with_context(|| {
            format!(
                "Failed to connect to cmux socket at {}",
                self.socket_path.display()
            )
        })
    }

    fn write_request<P>(&self, stream: &mut UnixStream, method: &str, params: &P) -> Result<()>
    where
        P: Serialize,
    {
        let request = RpcRequest { method, params };
        serde_json::to_writer(&mut *stream, &request)
            .with_context(|| format!("Failed to serialize cmux request for {}", method))?;
        stream
            .write_all(b"\n")
            .with_context(|| format!("Failed to write cmux request for {}", method))?;
        stream
            .flush()
            .with_context(|| format!("Failed to flush cmux request for {}", method))?;
        Ok(())
    }

    fn read_response(&self, stream: UnixStream, method: &str) -> Result<String> {
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .with_context(|| format!("Failed to read cmux response for {}", method))?;
        if line.trim().is_empty() {
            anyhow::bail!("cmux {} returned an empty response", method);
        }
        Ok(line)
    }
}

fn decode_response<T>(method: &str, raw_response: &str) -> Result<T>
where
    T: DeserializeOwned,
{
    let response: RpcResponse<T> = serde_json::from_str(raw_response).with_context(|| {
        format!(
            "Failed to parse cmux response for {}: {}",
            method,
            raw_response.trim()
        )
    })?;
    extract_result(method, raw_response.trim(), response)
}

fn extract_result<T>(method: &str, raw_response: &str, response: RpcResponse<T>) -> Result<T> {
    if !response.ok {
        if let Some(error) = response.error {
            anyhow::bail!("cmux {} failed: {}", method, error.display_message());
        }
        anyhow::bail!("cmux {} failed: {}", method, raw_response);
    }

    if let Some(result) = response.result {
        return Ok(result);
    }

    anyhow::bail!("cmux {} returned no result: {}", method, raw_response);
}

impl RpcFailure {
    fn display_message(&self) -> String {
        if let Some(message) = &self.message {
            return message.clone();
        }
        Value::Object(self.extra.clone()).to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[test]
    fn decode_response_returns_workspace_id_with_null_id() {
        let raw = r#"{"ok":true,"id":null,"result":{"workspace_id":"abc-123"}}"#;

        let result: CreateWorkspaceResult = decode_response("workspace.create", raw).unwrap();

        assert_eq!(result.workspace_id.as_deref(), Some("abc-123"));
    }

    #[test]
    fn decode_response_rejects_invalid_json() {
        let err =
            decode_response::<CreateWorkspaceResult>("workspace.create", "{not-json}").unwrap_err();

        assert!(
            err.to_string()
                .contains("Failed to parse cmux response for workspace.create")
        );
    }

    #[test]
    fn extract_result_prefers_structured_error_message() {
        let response = RpcResponse::<IgnoredAny> {
            ok: false,
            result: None,
            error: Some(RpcFailure {
                message: Some("denied".to_string()),
                extra: serde_json::Map::new(),
            }),
        };

        let err = extract_result("workspace.create", "{}", response).unwrap_err();

        assert_eq!(err.to_string(), "cmux workspace.create failed: denied");
    }

    #[test]
    fn extract_result_rejects_missing_result() {
        let response = RpcResponse::<CreateWorkspaceResult> {
            ok: true,
            result: None,
            error: None,
        };

        let err = extract_result("workspace.create", "{}", response).unwrap_err();

        assert!(err.to_string().contains("returned no result"));
    }

    #[test]
    fn from_env_prefers_env_override() {
        let _guard = env_lock().lock().unwrap();
        let old_socket = env::var_os(CMUX_SOCKET_PATH_ENV);
        unsafe {
            env::set_var(CMUX_SOCKET_PATH_ENV, "/tmp/cmux-test.sock");
        }

        let client = CmuxClient::from_env().unwrap();

        restore_var(CMUX_SOCKET_PATH_ENV, old_socket);
        assert_eq!(client.socket_path, PathBuf::from("/tmp/cmux-test.sock"));
    }

    #[test]
    fn from_env_errors_when_env_is_missing() {
        let _guard = env_lock().lock().unwrap();
        let old_socket = env::var_os(CMUX_SOCKET_PATH_ENV);
        unsafe {
            env::remove_var(CMUX_SOCKET_PATH_ENV);
        }

        let err = CmuxClient::from_env().unwrap_err();

        restore_var(CMUX_SOCKET_PATH_ENV, old_socket);
        assert!(err.to_string().contains("CMUX_SOCKET_PATH is not set"));
    }

    fn restore_var(key: &str, value: Option<OsString>) {
        unsafe {
            if let Some(value) = value {
                env::set_var(key, value);
            } else {
                env::remove_var(key);
            }
        }
    }
}
