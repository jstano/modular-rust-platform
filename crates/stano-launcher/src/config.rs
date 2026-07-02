use stano_security::JwtConfig;

#[derive(Clone, Debug)]
pub struct BootstrapConfig {
    pub port: u16,
    pub jwt_config: JwtConfig,
    pub cors_origins: Vec<String>,
}
