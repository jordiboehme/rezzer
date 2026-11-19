// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The supported model: files, checksums, chat framing and image rules.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// One model file with its expected size and SHA-256.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelFile {
    pub name: &'static str,
    pub size: u64,
    pub sha256: &'static str,
}

/// How the model sizes an image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageRule {
    pub multiple: u32,
    pub min_tokens: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelSpec {
    pub id: &'static str,
    pub repo: &'static str,
    pub revision: &'static str,
    pub licence: &'static str,
    pub model: ModelFile,
    pub mmproj: ModelFile,
    /// Chat framing around the user prompt; `{MEDIA}` and `{PROMPT}` are replaced.
    pub template: &'static str,
    pub image: ImageRule,
}

pub const QWEN35_4B: ModelSpec = ModelSpec {
    id: "qwen3.5-4b-q4_k_m",
    repo: "unsloth/Qwen3.5-4B-GGUF",
    revision: "e87f176479d0855a907a41277aca2f8ee7a09523",
    licence: "Apache-2.0",
    model: ModelFile {
        name: "Qwen3.5-4B-Q4_K_M.gguf",
        size: 2_740_937_888,
        sha256: "00fe7986ff5f6b463e62455821146049db6f9313603938a70800d1fb69ef11a4",
    },
    mmproj: ModelFile {
        name: "mmproj-F16.gguf",
        size: 672_423_616,
        sha256: "cd88edcf8d031894960bb0c9c5b9b7e1fea6ebee02b9f7ce925a00d12891f864",
    },
    template: "<|im_start|>user\n{MEDIA}{PROMPT}<|im_end|>\n<|im_start|>assistant\n<think>\n\n</think>\n\n",
    image: ImageRule {
        multiple: 32,
        min_tokens: 1024,
    },
};

/// One file on Hugging Face, pinned by revision, with the name it gets in the model folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HubFile {
    pub repo: &'static str,
    pub revision: &'static str,
    /// The file name in the repository.
    pub remote: &'static str,
    /// The file name in the local model folder.
    pub local: &'static str,
    pub size: u64,
    pub sha256: &'static str,
    pub licence: &'static str,
    pub licensor: &'static str,
}

/// PP-DocLayoutV3, the layout detector (feature `detect`).
pub const LAYOUT_V3: HubFile = HubFile {
    repo: "PaddlePaddle/PP-DocLayoutV3_safetensors",
    revision: "e4f489dc536556fc1be02b973ba06756e0d4a2ac",
    remote: "model.safetensors",
    local: "pp-doclayoutv3.safetensors",
    size: 133_270_468,
    sha256: "5ea422c6cc5fe759a47e1357c35639b58173508e025a3131cbe4b6ac59e2b85e",
    licence: "Apache-2.0",
    licensor: "PaddlePaddle Authors",
};

/// PP-OCRv6 small det, the text line detector (feature `detect`).
pub const TEXT_DET_V6: HubFile = HubFile {
    repo: "PaddlePaddle/PP-OCRv6_small_det_onnx",
    revision: "28fe5895c24fd108c19eb3e8479f4ab385fbfc62",
    remote: "inference.onnx",
    local: "pp-ocrv6-small-det.onnx",
    size: 9_880_512,
    sha256: "d73e0058b7a8086bbd57f3d10b8bcd4ff95363f67e06e2762b5e814fe9c9410e",
    licence: "Apache-2.0",
    licensor: "PaddlePaddle Authors",
};

impl ModelSpec {
    /// The model and the mmproj file as Hugging Face files.
    pub const fn hub_files(&self) -> [HubFile; 2] {
        [self.hub_file(self.model), self.hub_file(self.mmproj)]
    }

    const fn hub_file(&self, f: ModelFile) -> HubFile {
        HubFile {
            repo: self.repo,
            revision: self.revision,
            remote: f.name,
            local: f.name,
            size: f.size,
            sha256: f.sha256,
            licence: self.licence,
            licensor: "Qwen team, Alibaba Cloud",
        }
    }
}

/// Every file a caller downloads: the Qwen model, its mmproj and the two detectors.
pub const ALL_FILES: [HubFile; 4] = {
    let [model, mmproj] = QWEN35_4B.hub_files();
    [model, mmproj, LAYOUT_V3, TEXT_DET_V6]
};

/// The four model files in one folder, under their local names (`HubFile::local`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelFiles {
    pub model: PathBuf,
    pub mmproj: PathBuf,
    pub layout: PathBuf,
    pub text_det: PathBuf,
}

impl ModelFiles {
    /// The paths of [`ALL_FILES`] in `dir`.
    pub fn in_dir(dir: &Path) -> Self {
        let [model, mmproj, layout, text_det] = ALL_FILES;
        Self {
            model: dir.join(model.local),
            mmproj: dir.join(mmproj.local),
            layout: dir.join(layout.local),
            text_det: dir.join(text_det.local),
        }
    }
}

/// The pinned `llama-cpp-2` version; keep equal to the workspace pin. Default code, so the
/// scorecard check can read it without the `llama` feature.
pub const LLAMA_CPP_2_VERSION: &str = "0.1.161";

/// The pinned `candle-core` and `candle-nn` version; keep equal to the workspace pins.
pub const CANDLE_VERSION: &str = "0.11.1";

/// The download URL of a file, pinned to its revision.
pub fn hub_url(f: &HubFile) -> String {
    format!(
        "https://huggingface.co/{}/resolve/{}/{}",
        f.repo, f.revision, f.remote
    )
}

/// The version of the image preprocessing; part of the result fingerprint.
pub const PREPROCESS_VERSION: u32 = 1;

/// The lower-case hex SHA-256 of a file, read in pieces.
pub fn file_sha256(path: &Path) -> std::io::Result<String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

/// The lower-case hex SHA-256 of a prompt.
pub fn prompt_sha256(prompt: &str) -> String {
    let digest = Sha256::digest(prompt.as_bytes());
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qwen_spec_values() {
        let s = QWEN35_4B;
        assert_eq!(s.model.size, 2_740_937_888);
        assert_eq!(s.mmproj.size, 672_423_616);
        assert_eq!(
            s.model.sha256,
            "00fe7986ff5f6b463e62455821146049db6f9313603938a70800d1fb69ef11a4"
        );
        assert_eq!(
            s.mmproj.sha256,
            "cd88edcf8d031894960bb0c9c5b9b7e1fea6ebee02b9f7ce925a00d12891f864"
        );
        assert_eq!(s.revision, "e87f176479d0855a907a41277aca2f8ee7a09523");
        assert_eq!(s.image.multiple, 32);
        assert_eq!(s.image.min_tokens, 1024);
        assert!(s.template.contains("{MEDIA}{PROMPT}<|im_end|>"));
        assert!(s.template.ends_with("<think>\n\n</think>\n\n"));
    }

    #[test]
    fn layout_and_det_values() {
        assert_eq!(
            LAYOUT_V3,
            HubFile {
                repo: "PaddlePaddle/PP-DocLayoutV3_safetensors",
                revision: "e4f489dc536556fc1be02b973ba06756e0d4a2ac",
                remote: "model.safetensors",
                local: "pp-doclayoutv3.safetensors",
                size: 133_270_468,
                sha256: "5ea422c6cc5fe759a47e1357c35639b58173508e025a3131cbe4b6ac59e2b85e",
                licence: "Apache-2.0",
                licensor: "PaddlePaddle Authors",
            }
        );
        assert_eq!(
            TEXT_DET_V6,
            HubFile {
                repo: "PaddlePaddle/PP-OCRv6_small_det_onnx",
                revision: "28fe5895c24fd108c19eb3e8479f4ab385fbfc62",
                remote: "inference.onnx",
                local: "pp-ocrv6-small-det.onnx",
                size: 9_880_512,
                sha256: "d73e0058b7a8086bbd57f3d10b8bcd4ff95363f67e06e2762b5e814fe9c9410e",
                licence: "Apache-2.0",
                licensor: "PaddlePaddle Authors",
            }
        );
        let [model, mmproj] = QWEN35_4B.hub_files();
        assert_eq!(model.repo, "unsloth/Qwen3.5-4B-GGUF");
        assert_eq!(model.remote, "Qwen3.5-4B-Q4_K_M.gguf");
        assert_eq!(model.local, "Qwen3.5-4B-Q4_K_M.gguf");
        assert_eq!(mmproj.remote, "mmproj-F16.gguf");
        assert_eq!(mmproj.local, "mmproj-F16.gguf");
        assert_eq!(mmproj.size, 672_423_616);
        assert_eq!(model.licence, "Apache-2.0");
        assert_eq!(model.licensor, "Qwen team, Alibaba Cloud");
        assert_eq!(mmproj.licensor, "Qwen team, Alibaba Cloud");
        assert_eq!(model.revision, QWEN35_4B.revision);
    }

    #[test]
    fn all_files_has_four_distinct_local_names() {
        let mut names: Vec<&str> = ALL_FILES.iter().map(|f| f.local).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), 4);
        // The total of the addendum, section 7.
        let total: u64 = ALL_FILES.iter().map(|f| f.size).sum();
        assert_eq!(total, 3_556_512_484);
    }

    #[test]
    fn hub_url_pins_the_revision() {
        assert_eq!(
            hub_url(&LAYOUT_V3),
            "https://huggingface.co/PaddlePaddle/PP-DocLayoutV3_safetensors/resolve/e4f489dc536556fc1be02b973ba06756e0d4a2ac/model.safetensors"
        );
        for f in &ALL_FILES {
            let url = hub_url(f);
            assert!(!url.contains("/resolve/main/"), "{url}");
            assert!(url.contains(&format!("/resolve/{}/", f.revision)), "{url}");
        }
    }

    #[cfg(feature = "detect")]
    #[test]
    fn embedded_v3_config_hash() {
        let digest = Sha256::digest(crate::detect::V3_CONFIG_JSON);
        let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(
            hex,
            "3cf834b91d23a756b1519bce4db42c09e852f3e35c35092dd5a3e253a50c071a"
        );
    }

    #[test]
    fn model_files_use_the_local_names() {
        let f = ModelFiles::in_dir(Path::new("/m"));
        assert_eq!(f.model, Path::new("/m/Qwen3.5-4B-Q4_K_M.gguf"));
        assert_eq!(f.mmproj, Path::new("/m/mmproj-F16.gguf"));
        assert_eq!(f.layout, Path::new("/m/pp-doclayoutv3.safetensors"));
        assert_eq!(f.text_det, Path::new("/m/pp-ocrv6-small-det.onnx"));
    }

    #[test]
    fn version_constants_match_the_pins() {
        let manifest = include_str!("../../../../Cargo.toml");
        assert!(manifest.contains(&format!(
            "llama-cpp-2 = {{ version = \"={LLAMA_CPP_2_VERSION}\""
        )));
        for name in ["candle-core", "candle-nn"] {
            assert!(
                manifest.contains(&format!("{name} = {{ version = \"={CANDLE_VERSION}\"")),
                "{name}"
            );
        }
    }

    #[test]
    fn file_hash_reads_the_file() {
        let path = std::env::temp_dir().join(format!("pagevision-hash-{}", std::process::id()));
        std::fs::write(&path, "abc").unwrap();
        let hash = file_sha256(&path);
        std::fs::remove_file(&path).unwrap();
        assert_eq!(
            hash.unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert!(file_sha256(Path::new("/nonexistent/file")).is_err());
    }

    #[test]
    fn prompt_hash_is_stable() {
        assert_eq!(
            prompt_sha256("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
