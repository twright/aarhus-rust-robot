use futures::StreamExt;
use futures::stream::BoxStream;
use paho_mqtt as mqtt;
use paho_mqtt::Message;
use r2r;
use r2r::Publisher;
use r2r::QosProfile;
use r2r::qos::DurabilityPolicy as QosDurabilityPolicy;
use r2r::qos::HistoryPolicy as QosHistoryPolicy;
use r2r::qos::ReliabilityPolicy as QosReliabilityPolicy;
use serde::Deserialize;
use serde::Serialize;
use tokio::select;
use tokio::sync::oneshot;
use tracing::info;
// Removed incorrect import of std::fmt::Result.
use serde::de::{self, Deserializer};
use std::time::Duration;
use tracing::debug;
use tracing::error;
use tracing::instrument;
use tracing::warn;
use uuid::Uuid;

const MQTT_QOS: i32 = 1;
pub const SCAN_TOPIC: Topic = Topic {
    ros_name: "/scan_safe",
    mqtt_name: "/Scan",
};
pub const SPIN_TOPIC: Topic = Topic {
    ros_name: "/spin_config",
    mqtt_name: "/spin_config",
};

pub const TOPICS: [Topic; 2] = [SCAN_TOPIC, SPIN_TOPIC];

#[derive(Clone)]
pub struct Topic {
    pub ros_name: &'static str,
    pub mqtt_name: &'static str,
}
impl Copy for Topic {}

impl Topic {
    #[allow(dead_code)]
    fn ros_topic(topic: &str) -> Option<Topic> {
        TOPICS.iter().cloned().find(|&x| x.ros_name == topic)
    }

    #[allow(dead_code)]
    fn mqtt_topic(topic: &str) -> Option<Topic> {
        TOPICS.iter().cloned().find(|&x| x.mqtt_name == topic)
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(untagged)]
enum ROS2MQTTData {
    LaserScan(r2r::sensor_msgs::msg::LaserScan),
}

#[derive(Serialize, Debug, Clone)]
#[serde(untagged)]
enum MQTT2ROSData {
    SpinCommands(MSpinCommands),
}

/* Implement a custom deserializer for MQTT2ROSData
 * This added stricter validation than the default derived one, which does not
 * check for the presence of required fields in the SpinCommands message and
 * allows for extra fields.
 */
impl<'de> Deserialize<'de> for MQTT2ROSData {
    fn deserialize<D>(deserializer: D) -> Result<MQTT2ROSData, D::Error>
    where
        D: Deserializer<'de>,
    {
        // Deserialize into the helper struct. This will enforce that exactly
        // the fields "commands" (an array of objects each with "omega" and "duration")
        // and "period" are present.
        let helper = SpinCommandsHelper::deserialize(deserializer).map_err(de::Error::custom)?;
        // Convert the helper to your ROS message type.
        let spin_cmds = MSpinCommands::try_from(helper).map_err(de::Error::custom)?;
        Ok(MQTT2ROSData::SpinCommands(spin_cmds))
    }
}

async fn ros_node_actor(ros_namespace: &str) -> Result<(), r2r::Error> {
    let context = r2r::Context::create()?;
    let mut node = r2r::Node::create(
        context,
        format!("robosapiens_rosmqttbridge_{}", Uuid::new_v4().as_simple()).as_str(),
        ros_namespace,
    )?;

    let sensor_qos = QosProfile {
        // Keep last 5 messages, typical for sensor data
        history: QosHistoryPolicy::KeepLast,
        // Set depth to 5
        depth: 5,
        // Allow best effort delivery for low-latency sensor data
        reliability: QosReliabilityPolicy::BestEffort,
        // Volatile durability since historical data is not required
        durability: QosDurabilityPolicy::Volatile,

        deadline: QosProfile::default().deadline,
        lifespan: QosProfile::default().lifespan,
        liveliness: QosProfile::default().liveliness,
        liveliness_lease_duration: QosProfile::default().liveliness_lease_duration,
        avoid_ros_namespace_conventions: false,
    };

    debug!("Subscribing to scan safe");
    let sub = node.subscribe::<r2r::sensor_msgs::msg::LaserScan>("/scan", sensor_qos.clone())?;
    debug!("Subscribed to scan safe");

    // let publisher =
    // node.create_publisher::<MSpinCommands>(SPIN_TOPIC.ros_name, QosProfile::default())?;

    let fut = async move {
        loop {
            debug!("Spinning ROS node");
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            node.spin_once(std::time::Duration::from_millis(10));
        }
    };

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_log::test;
}
