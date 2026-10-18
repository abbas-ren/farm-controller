use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
#[serde(rename = "state")]
pub enum DeviceState {
    #[serde(rename = "free")]
    Free,
    #[serde(rename = "busy")]
    Busy,
    #[serde(rename = "faulty")]
    Faulty,
    #[serde(rename = "not_reachable")]
    NotReachable,
}

impl ToString for DeviceState {
    fn to_string(&self) -> String {
        match self {
            DeviceState::Free => "free".to_string(),
            DeviceState::Busy => "busy".to_string(),
            DeviceState::Faulty => "faulty".to_string(),
            DeviceState::NotReachable => "not_reachable".to_string(),
        }
    }
}

impl From<&String> for DeviceState {
    fn from(s: &String) -> Self {
        match s.as_str() {
            "free" => DeviceState::Free,
            "busy" => DeviceState::Busy,
            "faulty" => DeviceState::Faulty,
            "not_reachable" => DeviceState::NotReachable,
            _ => panic!("Invalid device state: {}", s),
        }
    }
}
