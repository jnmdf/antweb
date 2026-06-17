pub const MYSQL_URL: &str =
    "mysql://root:123456@localhost/antweb?socket=/opt/local/var/run/mysql8/mysqld.sock";

pub const BIND_ADDR: &str = "0.0.0.0:3000";
pub const TLS_CERT: &str = "certs/cert.pem";
pub const TLS_KEY: &str = "certs/key.pem";
pub const UPLOAD_MAX_BODY: usize = 20 * 1024 * 1024 * 1024;
