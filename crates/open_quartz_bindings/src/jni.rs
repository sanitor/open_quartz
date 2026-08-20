use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use open_quartz::{Environment, OpenQuartz, Player, Project};

pub struct JniHandleTable {
    clients: HashMap<u64, OpenQuartz>,
    projects: HashMap<u64, Project>,
    players: HashMap<u64, Player>,
    next: u64,
}

impl Default for JniHandleTable {
    fn default() -> Self {
        Self {
            clients: HashMap::new(),
            projects: HashMap::new(),
            players: HashMap::new(),
            next: 1,
        }
    }
}

impl JniHandleTable {
    pub fn create_client(&mut self) -> u64 {
        let handle = self.allocate();
        self.clients
            .insert(handle, OpenQuartz::new(Environment::headless()));
        handle
    }

    pub fn release_client(&mut self, handle: u64) -> bool {
        self.clients.remove(&handle).is_some()
    }

    pub fn create_project(&mut self, client: u64, name: &str) -> Result<u64, String> {
        let sdk = self
            .clients
            .get(&client)
            .ok_or_else(|| "Client handle is stale".to_owned())?
            .clone();
        let handle = self.allocate();
        self.projects.insert(handle, sdk.create_project(name));
        Ok(handle)
    }

    pub fn create_player(&mut self, project: u64) -> Result<u64, String> {
        let sdk = self
            .clients
            .values()
            .next()
            .ok_or_else(|| "Client handle is stale".to_owned())?
            .clone();
        let project_value = self
            .projects
            .get(&project)
            .ok_or_else(|| "Project handle is stale".to_owned())?;
        let player = sdk
            .player(project_value.graph())
            .build()
            .map_err(|error| error.to_json())?;
        let handle = self.allocate();
        self.players.insert(handle, player);
        Ok(handle)
    }

    pub fn with_player<T>(
        &mut self,
        handle: u64,
        operation: impl FnOnce(&mut Player) -> Result<T, String>,
    ) -> Result<T, String> {
        operation(
            self.players
                .get_mut(&handle)
                .ok_or_else(|| "Player handle is stale".to_owned())?,
        )
    }

    pub fn release_player(&mut self, handle: u64) -> bool {
        self.players.remove(&handle).is_some()
    }

    fn allocate(&mut self) -> u64 {
        let handle = self.next;
        self.next = self
            .next
            .checked_add(1)
            .expect("JNI handle table exhausted");
        handle
    }
}

pub type SharedJniHandleTable = Arc<Mutex<JniHandleTable>>;

pub fn new_handle_table() -> SharedJniHandleTable {
    Arc::new(Mutex::new(JniHandleTable::default()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aggregate_handles_are_released_and_stale_handles_rejected() {
        let mut table = JniHandleTable::default();
        let client = table.create_client();
        let project = table.create_project(client, "JNI").unwrap();
        let player = table.create_player(project).unwrap();
        assert!(table
            .with_player(player, |player| player
                .play()
                .map_err(|error| error.to_json()))
            .is_ok());
        assert!(table.release_player(player));
        assert!(table
            .with_player(player, |player| player
                .stop()
                .map_err(|error| error.to_json()))
            .is_err());
        assert!(table.release_client(client));
    }
}
