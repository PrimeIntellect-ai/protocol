use crate::plugins::newest_task::NewestTaskPlugin;
use alloy::primitives::Address;
use anyhow::Result;
use shared::models::task::Task;
use std::sync::Arc;

use crate::{
    models::node::{NodeStatus, OrchestratorNode},
    plugins::node_groups::NodeGroupsPlugin,
    plugins::webhook::WebhookPlugin,
};

pub(crate) mod newest_task;
pub(crate) mod node_groups;
pub(crate) mod webhook;

#[derive(Clone)]
pub(crate) enum StatusUpdatePlugin {
    NodeGroupsPlugin(Arc<NodeGroupsPlugin>),
    WebhookPlugin(WebhookPlugin),
}

impl StatusUpdatePlugin {
    pub(crate) async fn handle_status_change(
        &self,
        node: &OrchestratorNode,
        status: &NodeStatus,
    ) -> Result<()> {
        match self {
            StatusUpdatePlugin::NodeGroupsPlugin(plugin) => plugin.handle_status_change(node).await,
            StatusUpdatePlugin::WebhookPlugin(plugin) => {
                plugin.handle_status_change(node, status).await
            }
        }
    }
}

impl From<Arc<NodeGroupsPlugin>> for StatusUpdatePlugin {
    fn from(plugin: Arc<NodeGroupsPlugin>) -> Self {
        StatusUpdatePlugin::NodeGroupsPlugin(plugin)
    }
}

impl From<&Arc<NodeGroupsPlugin>> for StatusUpdatePlugin {
    fn from(plugin: &Arc<NodeGroupsPlugin>) -> Self {
        StatusUpdatePlugin::NodeGroupsPlugin(plugin.clone())
    }
}

impl From<WebhookPlugin> for StatusUpdatePlugin {
    fn from(plugin: WebhookPlugin) -> Self {
        StatusUpdatePlugin::WebhookPlugin(plugin)
    }
}

impl From<&WebhookPlugin> for StatusUpdatePlugin {
    fn from(plugin: &WebhookPlugin) -> Self {
        StatusUpdatePlugin::WebhookPlugin(plugin.clone())
    }
}

#[derive(Clone)]
pub(crate) enum SchedulerPlugin {
    NodeGroupsPlugin(Arc<NodeGroupsPlugin>),
    NewestTaskPlugin(NewestTaskPlugin),
}

impl SchedulerPlugin {
    pub(crate) async fn filter_tasks(
        &self,
        tasks: &[Task],
        node_address: &Address,
    ) -> Result<Vec<Task>> {
        match self {
            SchedulerPlugin::NodeGroupsPlugin(plugin) => {
                plugin.filter_tasks(tasks, node_address).await
            }
            SchedulerPlugin::NewestTaskPlugin(plugin) => plugin.filter_tasks(tasks).await,
        }
    }
}

impl From<Arc<NodeGroupsPlugin>> for SchedulerPlugin {
    fn from(plugin: Arc<NodeGroupsPlugin>) -> Self {
        SchedulerPlugin::NodeGroupsPlugin(plugin)
    }
}

impl From<&Arc<NodeGroupsPlugin>> for SchedulerPlugin {
    fn from(plugin: &Arc<NodeGroupsPlugin>) -> Self {
        SchedulerPlugin::NodeGroupsPlugin(plugin.clone())
    }
}

impl From<NewestTaskPlugin> for SchedulerPlugin {
    fn from(plugin: NewestTaskPlugin) -> Self {
        SchedulerPlugin::NewestTaskPlugin(plugin)
    }
}

impl From<&NewestTaskPlugin> for SchedulerPlugin {
    fn from(plugin: &NewestTaskPlugin) -> Self {
        SchedulerPlugin::NewestTaskPlugin(plugin.clone())
    }
}
