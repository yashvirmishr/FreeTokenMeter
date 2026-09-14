/// Freebuff provider stub.
/// 
/// Not yet implemented. Reports NOT CONNECTED until a real integration
/// source is confirmed.

pub struct FreebuffProvider;

#[allow(dead_code)]
impl FreebuffProvider {
    pub fn new() -> Self {
        Self
    }

    pub fn get_status(&self) -> (bool, String, Option<String>) {
        (false, "NOT CONNECTED".to_string(), Some("Freebuff integration not yet implemented.".to_string()))
    }
}
