// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The supported model: files, checksums, chat framing and image rules.

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

/// The version of the image preprocessing; part of the result fingerprint.
pub const PREPROCESS_VERSION: u32 = 1;

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
    fn prompt_hash_is_stable() {
        assert_eq!(
            prompt_sha256("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
