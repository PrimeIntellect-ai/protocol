use iroh::SecretKey;
use rand_v8::Rng;
use rand_v8::{rngs::StdRng, SeedableRng};
use std::error::Error;

/// Generate a random seed
pub fn generate_random_seed() -> u64 {
    rand_v8::thread_rng().gen()
}

// Generate an Iroh node ID from a seed
pub fn generate_iroh_node_id_from_seed(seed: u64) -> Result<String, Box<dyn Error>> {
    // Create a deterministic RNG from the seed
    let mut rng = StdRng::seed_from_u64(seed);

    // Generate the secret key using Iroh's method
    // This matches exactly how it's done in your Node implementation
    let secret_key = SecretKey::generate(&mut rng);

    // Get the node ID (public key) as a string
    let node_id = secret_key.public().to_string();

    Ok(node_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_random_seed() {
        let seed1 = generate_random_seed();
        let seed2 = generate_random_seed();

        assert_ne!(seed1, seed2);
    }

    #[test]
    fn test_known_generation() {
        let seed: u32 = 848364385;
        let result = generate_iroh_node_id_from_seed(seed as u64).unwrap();
        assert_eq!(
            result,
            "6ba970180efbd83909282ac741085431f54aa516e1783852978bd529a400d0e9"
        );
        assert_eq!(result.len(), 64);
    }

    #[test]
    fn test_deterministic_generation() {
        // Same seed should generate same node_id
        let seed = generate_random_seed();
        let result1 = generate_iroh_node_id_from_seed(seed).unwrap();
        let result2 = generate_iroh_node_id_from_seed(seed).unwrap();

        assert_eq!(result1, result2);
    }
}
