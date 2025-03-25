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

async fn ros_node_actor(ros_namespace: &str) -> Result<(), r2r::Error> {
    let context = r2r::Context::create()?;
    let mut node = r2r::Node::create(
        context,
        format!("rust_aarhus_{}", Uuid::new_v4().as_simple()).as_str(),
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

    debug!("Subscribing to scan");
    let sub = node.subscribe::<r2r::sensor_msgs::msg::LaserScan>("/scan", sensor_qos.clone())?;
    debug!("Subscribed to scan");

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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::from_default_env())
        .init();

    let ros_namespace = std::env::var("ROS_NAMESPACE").unwrap_or_else(|_| "".to_string());

    Ok(ros_node_actor(ros_namespace.as_str()).await?)
}
