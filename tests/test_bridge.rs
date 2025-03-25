use futures::StreamExt;
use paho_mqtt as mqtt;
use std::time::Duration;
use tokio::time::sleep;
use tracing::info;

// Import the bridge module from your crate.
use r2r::QosProfile;
use r2r::qos::DurabilityPolicy as QosDurabilityPolicy;
use r2r::qos::HistoryPolicy as QosHistoryPolicy;
use r2r::qos::ReliabilityPolicy as QosReliabilityPolicy;
use r2r::spin_interfaces::msg::SpinCommand as MSpinCommand;
use r2r::spin_interfaces::msg::SpinPeriodicCommands as MSpinCommands;
use ros2mqttbridge::SPIN_TOPIC;
use ros2mqttbridge::bridge;
use test_log::test;
use testcontainers::core::IntoContainerPort;
use testcontainers::core::WaitFor;
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, GenericImage, ImageExt};
use tracing::instrument;

#[instrument(level = tracing::Level::INFO)]
async fn start_emqx() -> ContainerAsync<GenericImage> {
    let image = GenericImage::new("emqx/emqx", "5.8.3")
        .with_wait_for(WaitFor::message_on_stdout("EMQX 5.8.3 is running now!"))
        .with_exposed_port(1883_u16.tcp())
        .with_startup_timeout(Duration::from_secs(30));
    // .with_mount(Mount::bind_mount("/tmp/emqx_logs", "/opt/emqx/log"));

    image
        .start()
        .await
        .expect("Failed to start EMQX test container")
}

const MQTT_QOS: i32 = 1;

#[test(tokio::test)]
async fn test_mqtt_to_ros() {
    // Start Docker client using testcontainers.
    let emqx_container = start_emqx().await;
    let mqtt_port = emqx_container.get_host_port_ipv4(1883).await.unwrap();
    let mqtt_uri = format!("tcp://localhost:{}", mqtt_port);
    info!("Started Mqtt on {}", mqtt_port);
    println!("Started Mqtt on {}", mqtt_port);

    let ros_context = r2r::Context::create().unwrap();
    let test_ros_namespace = "test_bridge_integration";
    let mut node =
        r2r::Node::create(ros_context, "test_bridge_integration", test_ros_namespace).unwrap();
    let mut spin_sub = node
        .subscribe::<MSpinCommands>(SPIN_TOPIC.ros_name, r2r::QosProfile::default())
        .unwrap();

    let spinner = tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_millis(100)).await;
            node.spin_once(std::time::Duration::from_millis(0));
        }
    });

    // Spawn the bridge. (Your bridge function currently hardcodes "tcp://localhost:1883"
    // so ensure that mqtt_uri points to the same broker if necessary.)
    let mqtt_uri_clone = mqtt_uri.clone();
    let bridge_handle = tokio::spawn(async move {
        bridge(mqtt_uri_clone.as_str(), test_ros_namespace)
            .await
            .unwrap();
    });

    // Give the bridge time to start.
    sleep(Duration::from_secs(1)).await;

    // --- Test MQTT -> ROS side ---
    // Here we simulate sending a SpinCommands message via MQTT to be forwarded to ROS.
    let create_opts_pub = mqtt::CreateOptionsBuilder::new_v3()
        .server_uri(mqtt_uri.clone())
        .client_id("test_publisher")
        .finalize();
    let pub_client = mqtt::AsyncClient::new(create_opts_pub).unwrap();
    let connect_opts = mqtt::ConnectOptionsBuilder::new_v3()
        .keep_alive_interval(Duration::from_secs(30))
        .clean_session(false)
        .finalize();
    pub_client.connect(connect_opts.clone()).await.unwrap();

    // Create a dummy MSpinCommands message (adjust the fields as appropriate).
    let spin_cmds = vec![
        // Spin at 0.5 rad/s for 2 seconds.
        MSpinCommand {
            omega: 0.5,
            duration: 2.0,
        },
        // Spin at 1.0 rad/s for 1.5 seconds.
        MSpinCommand {
            omega: 1.0,
            duration: 1.5,
        },
        // Spin in reverse at -0.5 rad/s for 3 seconds.
        MSpinCommand {
            omega: -0.5,
            duration: 3.0,
        },
    ];
    let spin_cmd = MSpinCommands {
        commands: spin_cmds,
        period: 0.1,
    };
    let serialized = serde_json5::to_string(&spin_cmd).unwrap();
    let msg = mqtt::Message::new("/spin_config", serialized, MQTT_QOS);
    pub_client.publish(msg.clone()).await.unwrap();

    // Check the received message.
    let ros_message = spin_sub.next().await.unwrap();

    // Check that the received message matches the sent message.
    assert_eq!(ros_message, spin_cmd);

    // Shut down the bridge (abort its task).
    spinner.abort();
    bridge_handle.abort();
    pub_client.disconnect(None).await.unwrap();
    // tokio::time::sleep(Duration::from_secs(1)).await;
    info!("Shutting down test_mqtt_to_ros");
}

#[test(tokio::test)]
async fn test_ros_to_mqtt() {
    // Assume the MQTT test container is already running.
    // Retrieve the EMQX MQTT broker URI from your shared test container.
    let emqx_container = start_emqx().await;
    let mqtt_port = emqx_container.get_host_port_ipv4(1883).await.unwrap();
    let mqtt_uri = format!("tcp://localhost:{}", mqtt_port);
    info!("Started MQTT on {}", mqtt_uri);

    // Create a ROS node for publishing LaserScan messages.
    let ros_context = r2r::Context::create().unwrap();
    let test_ros_namespace = "test_bridge_integration_2";
    let mut node = r2r::Node::create(ros_context, "test_scan_pub", test_ros_namespace).unwrap();

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

    // Create a publisher for LaserScan messages on the "/scan_safe" topic.
    let scan_pub = node
        .create_publisher::<r2r::sensor_msgs::msg::LaserScan>("/scan_safe", sensor_qos)
        .unwrap();

    // Spin the ROS node so that callbacks are processed.
    let spinner = tokio::spawn(async move {
        loop {
            sleep(Duration::from_millis(100)).await;
            node.spin_once(Duration::from_millis(10));
        }
    });

    // Spawn the bridge with the MQTT URI and test ROS namespace.
    let mqtt_uri_clone = mqtt_uri.clone();
    let bridge_handle = tokio::spawn(async move {
        bridge(mqtt_uri_clone.as_str(), test_ros_namespace)
            .await
            .unwrap();
    });

    // Give the bridge time to start.
    sleep(Duration::from_secs(1)).await;

    // Setup an MQTT subscriber to receive the bridged LaserScan on topic "/Scan".
    let create_opts_sub = mqtt::CreateOptionsBuilder::new_v3()
        .server_uri(mqtt_uri.clone())
        .client_id("test_scan_subscriber")
        .finalize();
    let mut sub_client = mqtt::AsyncClient::new(create_opts_sub).unwrap();
    let connect_opts = mqtt::ConnectOptionsBuilder::new_v3()
        .keep_alive_interval(Duration::from_secs(30))
        .clean_session(false)
        .finalize();
    sub_client.connect(connect_opts.clone()).await.unwrap();
    sub_client.subscribe("/Scan", MQTT_QOS).await.unwrap();
    let mut mqtt_stream = sub_client.get_stream(25);

    // Create a dummy LaserScan message.
    // Fill in required fields; using r2r types for demonstration.
    let now = r2r::Clock::create(r2r::ClockType::RosTime)
        .unwrap()
        .get_now()
        .unwrap();
    let fake_scan = r2r::sensor_msgs::msg::LaserScan {
        header: r2r::std_msgs::msg::Header {
            stamp: r2r::builtin_interfaces::msg::Time {
                sec: now.as_secs() as i32,
                nanosec: now.subsec_nanos(),
            },
            frame_id: "laser_frame".to_string(),
        },
        angle_min: 0.0,
        angle_max: 3.14,
        angle_increment: 0.1,
        time_increment: 0.0,
        scan_time: 0.1,
        range_min: 0.0,
        range_max: 10.0,
        ranges: vec![1.0, 2.0, 3.0],
        intensities: vec![0.5, 0.6, 0.7],
    };

    // Publish the LaserScan message on the ROS topic "/scan_safe".
    scan_pub.publish(&fake_scan).unwrap();

    // Wait for the corresponding MQTT message on "/Scan".
    let mut received_msg = None;
    if let Some(Some(msg)) = mqtt_stream.next().await {
        received_msg = Some(msg);
    }
    let received_msg = received_msg.expect("No MQTT message received for LaserScan");

    info!(
        "Received MQTT message on /Scan: {}",
        received_msg.payload_str()
    );

    // Deserialize the MQTT message payload back into a LaserScan message.
    let received_scan: r2r::sensor_msgs::msg::LaserScan =
        serde_json5::from_str(&received_msg.payload_str())
            .expect("Failed to deserialize LaserScan");

    // Check that the received scan matches the published message.
    assert_eq!(received_scan.header.frame_id, fake_scan.header.frame_id);
    assert!((received_scan.angle_max - fake_scan.angle_max).abs() < 0.001);
    assert_eq!(received_scan.ranges, fake_scan.ranges);
    assert_eq!(received_scan.intensities, fake_scan.intensities);

    // Clean up the tasks.
    spinner.abort();
    bridge_handle.abort();
    sub_client.disconnect(None).await.unwrap();
    info!("Shutting down test_ros_to_mqtt");
}
