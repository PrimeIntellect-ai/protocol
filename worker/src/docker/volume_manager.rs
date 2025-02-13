use bollard::errors::Error as DockerError;
use bollard::volume::CreateVolumeOptions;
use bollard::Docker;
use std::collections::HashMap;

pub struct VolumeManager {
    storage_path: Option<String>,
    docker: Docker
}

impl VolumeManager {
    pub fn new(storage_path: Option<String>, docker: Docker) -> Result<Self, DockerError>{
        Ok(Self {
            storage_path,
            docker
        })

    }

    pub async fn create_volume(&self, name: &str) -> Result<(), DockerError> { 
        let name = format!("{}_data", name);    
        let path = format!(
                "{}/{}",
                self.storage_path.clone(),
                name.trim_start_matches('/')
            );
            std::fs::create_dir_all(&path)?;

            self.docker
                .create_volume(CreateVolumeOptions {
                    name: name,
                    driver: "local".to_string(),
                    driver_opts: HashMap::from([
                        ("type".to_string(), "none".to_string()),
                        ("o".to_string(), "bind".to_string()),
                        ("device".to_string(), path),
                    ]),
                    labels: HashMap::new(),
                })
                .await?;
        Ok(())
    }

    pub async fn delete_volume(){ 
        // TODO
     }

    pub async fn setup_shared_worker_volume(){
        // TODO: check if shared worker volume already exists 


    }

}
