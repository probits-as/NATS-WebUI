use log::{debug, error};
use reqwest;
use serde::{Deserialize, Serialize};

#[derive(Default, Clone, Serialize, Deserialize, Debug)]
pub struct App {
    pub servers: Vec<NatsServer>,
    pub clients: Vec<NatsClient>,
}

impl App {
    pub fn set_servers(&mut self, servers: Vec<NatsServer>) {
        self.servers = servers;
    }

    pub fn set_clients(&mut self, clients: Vec<NatsClient>) {
        self.clients = clients;
    }
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct NatsServer {
    pub id: Option<i64>,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub monitoring_port: u16,
    pub token: Option<String>,
    pub varz: Option<ServerVarz>,
    pub subjects: Vec<SubjectTreeNode>,
    pub publications: Vec<Publication>,
}

impl NatsServer {
    pub async fn get_varz(
        id: i64,
        host: String,
        port: u16,
        client: &reqwest::Client,
    ) -> Result<VarzBroadcastMessage, VarzError> {
        let response = client
            .get(&format!("http://{}:{}/varz", host, port))
            .send()
            .await?;

        response.error_for_status_ref()?;

        let json_text = response.text().await?;
        debug!("Raw VARZ JSON: {}", json_text);

        let varz: ServerVarz = serde_json::from_str(&json_text).map_err(|e| {
            error!("Failed to parse VARZ JSON: {:?}", e);
            VarzError::from(e)
        })?;

        Ok(VarzBroadcastMessage {
            server_id: id,
            varz,
        })
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct VarzBroadcastMessage {
    pub server_id: i64,
    varz: ServerVarz,
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
pub struct SubjectTreeNode {
    id: String,
    subject_str: String,
    subjects: Vec<SubjectTreeNode>,
    selected: bool,
}

impl SubjectTreeNode {
    pub fn flatten(&self) -> Vec<String> {
        let mut result = vec![self.subject_str.clone()];
        for child in &self.subjects {
            result.extend(child.flatten());
        }
        result
    }

    pub fn get_subscriptions(&self) -> Vec<String> {
        self.flatten()
    }
}

#[derive(Clone, Serialize, Deserialize, Debug, Eq, PartialEq)]
pub struct Publication {
    subject: String,
    message: String,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct ServerVarz {
    pub server_id: String,
    pub server_name: String,
    pub version: String,
    pub proto: i32,
    pub go: String,
    pub host: String,
    pub port: u16,
    pub max_connections: i64,
    pub ping_interval: i64,
    pub ping_max: i64,
    pub http_host: String,
    pub http_port: u16,
    pub https_port: u16,
    pub auth_timeout: i64,
    pub max_control_line: i64,
    pub max_payload: i64,
    pub max_pending: i64,
    #[serde(default)]
    pub cluster: Option<ClusterInfo>,
    #[serde(default)]
    pub gateway: Option<serde_json::Value>,
    #[serde(default)]
    pub leaf: Option<serde_json::Value>,
    #[serde(default)]
    pub mqtt: Option<MqttConfig>,
    #[serde(default)]
    pub websocket: Option<serde_json::Value>,
    #[serde(default)]
    pub jetstream: Option<JetstreamConfig>,
    pub tls_timeout: i64,
    pub write_deadline: i64,
    pub start: String,
    pub now: String,
    pub uptime: String,
    pub mem: i64,
    pub cores: i32,
    pub gomaxprocs: i32,
    pub cpu: f64,
    pub connections: i64,
    pub total_connections: i64,
    pub routes: i64,
    pub remotes: i64,
    pub leafnodes: i64,
    pub in_msgs: i64,
    pub out_msgs: i64,
    pub in_bytes: i64,
    pub out_bytes: i64,
    pub slow_consumers: i64,
    pub subscriptions: i64,
    pub http_req_stats: HttpReqStats,
    pub config_load_time: String,
    #[serde(default)]
    pub system_account: Option<String>,
    pub slow_consumer_stats: SlowConsumerStats,
    #[serde(default)]
    pub git_commit: Option<String>,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct ClusterInfo {
    #[serde(default)]
    pub addr: Option<String>,
    #[serde(default)]
    pub cluster_port: Option<u16>,
    #[serde(default)]
    pub auth_timeout: Option<i64>,
    #[serde(default)]
    pub tls_timeout: Option<i64>,
    #[serde(default)]
    pub tls_required: Option<bool>,
    #[serde(default)]
    pub tls_verify: Option<bool>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub urls: Option<Vec<String>>,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct MqttConfig {
    pub host: String,
    pub port: u16,
    pub tls_timeout: i64,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct JetstreamConfig {
    pub config: JetstreamServerConfig,
    pub stats: JetstreamStats,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct JetstreamServerConfig {
    pub max_memory: i64,
    pub max_storage: i64,
    pub store_dir: String,
    pub sync_interval: i64,
    pub compress_ok: bool,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct JetstreamStats {
    pub memory: i64,
    pub storage: i64,
    pub reserved_memory: i64,
    pub reserved_storage: i64,
    pub accounts: i64,
    pub ha_assets: i64,
    pub api: JetstreamApiStats,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct JetstreamApiStats {
    pub total: i64,
    pub errors: i64,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct HttpReqStats {
    #[serde(rename = "/varz")]
    pub varz: i64,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct SlowConsumerStats {
    pub clients: i64,
    pub routes: i64,
    pub gateways: i64,
    pub leafs: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NatsClient {
    pub id: Option<i64>,
    pub name: String,
    pub server_id: i64,
    pub subjects: Vec<SubjectTreeNode>,
    pub info: bool,
    pub ping: bool,
    pub pong: bool,
    pub ok: bool,
    pub err: bool,
    pub publ: bool,
    pub sub: bool,
    pub unsub: bool,
    pub connect: bool,
    pub msg: bool,
}

impl NatsClient {
    pub fn get_subscriptions(&self) -> Vec<String> {
        self.subjects
            .iter()
            .flat_map(|s| s.get_subscriptions())
            .collect()
    }
}

#[derive(Debug)]
pub enum VarzError {
    RequestError(reqwest::Error),
    JsonError(serde_json::Error),
}

impl From<reqwest::Error> for VarzError {
    fn from(err: reqwest::Error) -> Self {
        VarzError::RequestError(err)
    }
}

impl From<serde_json::Error> for VarzError {
    fn from(err: serde_json::Error) -> Self {
        VarzError::JsonError(err)
    }
}
