use anyhow::Result;
use std::ffi::{CStr, CString};
use std::fs::File;
use std::os::raw::{c_char, c_float, c_int, c_void};
use std::path::Path;
use tracing::{info, warn};

// Archive support for single-file index storage
use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use tar::{Archive, Builder};

/// Search result from vector index
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub vector_id: usize,
    pub distance: f32,
}

// FFI declarations for C++ wrapper functions
#[link(name = "spfresh_wrapper", kind = "static")]
unsafe extern "C" {
    fn spfresh_create_index(
        algo_type: *const c_char,
        value_type: *const c_char,
        dimension: c_int,
    ) -> *mut c_void;

    fn spfresh_add_vector(
        index: *mut c_void,
        vector: *const c_float,
        dimension: c_int,
    ) -> c_int;

    fn spfresh_build_index(
        index: *mut c_void,
        vectors: *const c_float,
        num_vectors: c_int,
        dimension: c_int,
    ) -> c_int;

    fn spfresh_search(
        index: *mut c_void,
        query: *const c_float,
        dimension: c_int,
        k: c_int,
        result_indices: *mut c_int,
        result_distances: *mut c_float,
    ) -> c_int;

    fn spfresh_save_index(index: *mut c_void, folder_path: *const c_char) -> c_int;

    fn spfresh_load_index(folder_path: *const c_char) -> *mut c_void;

    fn spfresh_get_num_vectors(index: *mut c_void) -> c_int;

    fn spfresh_get_dimension(index: *mut c_void) -> c_int;

    fn spfresh_set_parameter(
        index: *mut c_void,
        param_name: *const c_char,
        param_value: *const c_char,
    ) -> c_int;

    fn spfresh_destroy_index(index: *mut c_void);
}

/// SPFresh vector index
pub struct VectorIndex {
    index_type: String,
    vector_dim: usize,
    num_trees: usize,
    index_ptr: *mut c_void,
    vector_count: usize,
}

unsafe impl Send for VectorIndex {}
unsafe impl Sync for VectorIndex {}

impl VectorIndex {
    /// Create a new vector index
    pub fn new(index_type: String, vector_dim: usize, num_trees: usize) -> Self {
        info!(
            index_type = %index_type,
            vector_dim = vector_dim,
            num_trees = num_trees,
            "Creating new vector index"
        );

        Self {
            index_type,
            vector_dim,
            num_trees,
            index_ptr: std::ptr::null_mut(),
            vector_count: 0,
        }
    }

    /// Initialize the index
    pub fn initialize(&mut self) -> Result<()> {
        info!("Initializing SPFresh vector index");

        unsafe {
            let algo_type = CString::new(self.index_type.as_str())?;
            let value_type = CString::new("Float")?;

            self.index_ptr = spfresh_create_index(
                algo_type.as_ptr(),
                value_type.as_ptr(),
                self.vector_dim as c_int,
            );

            if self.index_ptr.is_null() {
                anyhow::bail!("Failed to create SPFresh index");
            }

            // Set index parameters
            self.set_param("DistCalcMethod", "L2")?;
            self.set_param("NumberOfThreads", "4")?;
            
            // BKT/KDT specific parameters
            if self.index_type == "BKT" {
                self.set_param("BKTNumber", &self.num_trees.to_string())?;
                self.set_param("BKTKmeansK", "32")?;
            } else if self.index_type == "KDT" {
                self.set_param("KDTNumber", &self.num_trees.to_string())?;
            }

            info!("✅ SPFresh index initialized successfully");
        }

        Ok(())
    }

    /// Set a parameter on the index
    fn set_param(&self, name: &str, value: &str) -> Result<()> {
        unsafe {
            let param_name = CString::new(name)?;
            let param_value = CString::new(value)?;

            let ret = spfresh_set_parameter(
                self.index_ptr,
                param_name.as_ptr(),
                param_value.as_ptr(),
            );

            if ret != 0 {
                warn!("Failed to set parameter {}={}", name, value);
            }
        }
        Ok(())
    }

    /// Add a vector to the index
    /// Returns the vector ID (sequential, starting from 0)
    pub fn add_vector(&mut self, vector: &[f32]) -> Result<usize> {
        if vector.len() != self.vector_dim {
            anyhow::bail!(
                "Vector dimension mismatch: expected {}, got {}",
                self.vector_dim,
                vector.len()
            );
        }

        if self.index_ptr.is_null() {
            anyhow::bail!("Index not initialized");
        }

        unsafe {
            let vector_id = spfresh_add_vector(
                self.index_ptr,
                vector.as_ptr(),
                self.vector_dim as c_int,
            );

            if vector_id < 0 {
                anyhow::bail!("Failed to add vector to index");
            }

            self.vector_count += 1;
            info!(vector_id = vector_id, total = self.vector_count, "Added vector to index");

            Ok(vector_id as usize)
        }
    }

    /// Build index from vectors (more efficient than adding one-by-one)
    pub fn build_from_vectors(&mut self, vectors: &[Vec<f32>]) -> Result<()> {
        if vectors.is_empty() {
            return Ok(());
        }

        if self.index_ptr.is_null() {
            anyhow::bail!("Index not initialized");
        }

        // Flatten vectors into contiguous array
        let num_vectors = vectors.len();
        let mut flat_vectors = Vec::with_capacity(num_vectors * self.vector_dim);
        
        for vec in vectors {
            if vec.len() != self.vector_dim {
                anyhow::bail!("Vector dimension mismatch");
            }
            flat_vectors.extend_from_slice(vec);
        }

        unsafe {
            let ret = spfresh_build_index(
                self.index_ptr,
                flat_vectors.as_ptr(),
                num_vectors as c_int,
                self.vector_dim as c_int,
            );

            if ret != 0 {
                anyhow::bail!("Failed to build index");
            }

            self.vector_count = num_vectors;
            info!(num_vectors = num_vectors, "Built index from vectors");
        }

        Ok(())
    }

    /// Search for k-nearest neighbors
    pub fn search(&self, query_vector: &[f32], k: usize) -> Result<Vec<SearchResult>> {
        if query_vector.len() != self.vector_dim {
            anyhow::bail!(
                "Query vector dimension mismatch: expected {}, got {}",
                self.vector_dim,
                query_vector.len()
            );
        }

        if self.index_ptr.is_null() {
            anyhow::bail!("Index not initialized");
        }

        unsafe {
            let mut result_indices = vec![0i32; k];
            let mut result_distances = vec![0.0f32; k];

            let count = spfresh_search(
                self.index_ptr,
                query_vector.as_ptr(),
                self.vector_dim as c_int,
                k as c_int,
                result_indices.as_mut_ptr(),
                result_distances.as_mut_ptr(),
            );

            if count < 0 {
                anyhow::bail!("Search failed");
            }

            let results: Vec<SearchResult> = (0..count as usize)
                .map(|i| SearchResult {
                    vector_id: result_indices[i] as usize,
                    distance: result_distances[i],
                })
                .collect();

            info!(query_results = count, k = k, "Search completed");

            Ok(results)
        }
    }

    /// Save index to a single tar.gz file
    pub fn save(&self, path: &Path) -> Result<()> {
        if self.index_ptr.is_null() {
            anyhow::bail!("Index not initialized");
        }

        info!("Saving index to {:?}", path);

        // Create temp directory for SPFresh to save folder structure
        let temp_dir = path.with_extension("tmp");
        if temp_dir.exists() {
            std::fs::remove_dir_all(&temp_dir)?;
        }
        std::fs::create_dir_all(&temp_dir)?;

        // Save to temp folder (SPFresh native format)
        unsafe {
            let temp_str = temp_dir.to_str().ok_or_else(|| anyhow::anyhow!("Invalid temp path"))?;
            let temp_cstr = CString::new(temp_str)?;

            let ret = spfresh_save_index(self.index_ptr, temp_cstr.as_ptr());

            if ret != 0 {
                std::fs::remove_dir_all(&temp_dir)?;
                anyhow::bail!("Failed to save index to temp folder");
            }
        }

        // Create tar.gz archive from temp folder
        let archive_file = File::create(path)?;
        let encoder = GzEncoder::new(archive_file, Compression::default());
        let mut tar = Builder::new(encoder);
        
        tar.append_dir_all(".", &temp_dir)?;
        tar.finish()?;

        // Cleanup temp folder
        std::fs::remove_dir_all(&temp_dir)?;

        info!("✅ Index saved successfully to single file");

        Ok(())
    }

    /// Load index from a single tar.gz file
    pub fn load(&mut self, path: &Path) -> Result<()> {
        if !path.exists() {
            anyhow::bail!("Index path does not exist: {:?}", path);
        }

        info!("Loading index from {:?}", path);

        // Create temp directory for extraction
        let temp_dir = path.with_extension("tmp");
        if temp_dir.exists() {
            std::fs::remove_dir_all(&temp_dir)?;
        }
        std::fs::create_dir_all(&temp_dir)?;

        // Extract tar.gz archive to temp folder
        let archive_file = File::open(path)?;
        let decoder = GzDecoder::new(archive_file);
        let mut tar = Archive::new(decoder);
        tar.unpack(&temp_dir)?;

        // Load from temp folder (SPFresh native format)
        unsafe {
            let temp_str = temp_dir.to_str().ok_or_else(|| anyhow::anyhow!("Invalid temp path"))?;
            let temp_cstr = CString::new(temp_str)?;

            let new_ptr = spfresh_load_index(temp_cstr.as_ptr());

            if new_ptr.is_null() {
                std::fs::remove_dir_all(&temp_dir)?;
                anyhow::bail!("Failed to load index from temp folder");
            }

            // Destroy old index if exists
            if !self.index_ptr.is_null() {
                spfresh_destroy_index(self.index_ptr);
            }

            self.index_ptr = new_ptr;

            // Update stats
            let num_vectors = spfresh_get_num_vectors(self.index_ptr);
            let dimension = spfresh_get_dimension(self.index_ptr);

            if num_vectors >= 0 {
                self.vector_count = num_vectors as usize;
            }
            if dimension >= 0 && dimension as usize != self.vector_dim {
                warn!(
                    "Loaded index dimension ({}) differs from configured ({})",
                    dimension, self.vector_dim
                );
                self.vector_dim = dimension as usize;
            }

            info!(
                num_vectors = self.vector_count,
                dimension = self.vector_dim,
                "✅ Index loaded successfully from single file"
            );
        }

        // Cleanup temp folder
        std::fs::remove_dir_all(&temp_dir)?;

        Ok(())
    }

    /// Get number of vectors in the index
    pub fn vector_count(&self) -> usize {
        self.vector_count
    }
}

impl Drop for VectorIndex {
    fn drop(&mut self) {
        if !self.index_ptr.is_null() {
            unsafe {
                spfresh_destroy_index(self.index_ptr);
            }
            info!("SPFresh index destroyed");
        }
    }
}
