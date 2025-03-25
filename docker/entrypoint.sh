#!/bin/bash
# Basic entrypoint for ROS / Colcon Docker containers

# Source ROS 2
source /opt/ros/humble/setup.bash
# echo "Sourced ROS 2 ${ROS_DISTRO}"

# Source the base workspace, if built
if [ -f /opt/setup.bash ]
then
  source /opt/setup.bash
  # echo "Sourced TurtleBot4 base workspace"
fi

export PATH=$HOME/.cargo/bin/:$PATH


# Execute the command passed into this entrypoint
exec "$@"
