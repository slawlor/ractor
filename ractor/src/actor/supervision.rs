// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

//! Supervision management logic

use std::{collections::HashMap, sync::Arc};

use tokio::sync::RwLock;

use super::{actor_cell::ActorCell, messages::SupervisionEvent};
use crate::ActorId;

/// A supervision tree
#[derive(Clone, Default)]
pub struct SupervisionTree {
    children: Arc<RwLock<HashMap<ActorId, ActorCell>>>,
    parents: Arc<RwLock<HashMap<ActorId, ActorCell>>>,
}

impl SupervisionTree {
    /// Push a child into the tere
    pub async fn insert_child(&self, child: ActorCell) {
        let mut guard = self.children.write().await;
        guard.insert(child.get_id(), child.clone());
    }

    /// Remove a specific actor from the supervision tree (e.g. actor died)
    pub async fn remove_child(&self, child: ActorCell) {
        let mut guard = self.children.write().await;
        let id = child.get_id();
        if guard.contains_key(&id) {
            guard.remove(&id);
        }
    }

    /// Push a parent into the tere
    pub async fn insert_parent(&self, parent: ActorCell) {
        let mut guard = self.parents.write().await;
        guard.insert(parent.get_id(), parent.clone());
    }

    /// Remove a specific actor from the supervision tree (e.g. actor died)
    pub async fn remove_parent(&self, parent: ActorCell) {
        let mut guard = self.parents.write().await;
        let id = parent.get_id();
        if guard.contains_key(&id) {
            guard.remove(&id);
        }
    }

    /// Terminate all your supervised children
    #[async_recursion::async_recursion]
    pub async fn terminate(&self) {
        let guard = self.children.read().await;

        for (_, child) in guard.iter() {
            child.terminate().await;
        }
    }

    /// Determine if the specified actor is a member of this supervision tree
    pub async fn is_supervisor_of(&self, id: ActorId) -> bool {
        self.children.read().await.contains_key(&id)
    }

    /// Determine if the specified actor is a parent of this actor
    pub async fn is_child_of(&self, id: ActorId) -> bool {
        self.parents.read().await.contains_key(&id)
    }

    /// Send a notification to all supervisors
    pub async fn notify_supervisors(
        &self,
        evt: SupervisionEvent,
    ) -> Result<(), tokio::sync::mpsc::error::SendError<SupervisionEvent>> {
        for (_, parent) in self.parents.read().await.iter() {
            parent.send_supervisor_evt(evt.clone())?;
        }
        Ok(())
    }
}
