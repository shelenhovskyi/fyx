//! Fyx (plan 6.5, `docs/fyx/platform/edit-predictions.md`): the FIM prompt format of a local
//! model by its family, for names upstream's exact match misses. `llama-server` reports its GGUF
//! file name (`qwen2.5-coder-1.5b-instruct-q8_0.gguf`), and Ollama tags carry sizes
//! (`deepseek-coder-v2:16b`); without a format the provider is silently off.

use language::EditPredictionPromptFormat;

/// Family prefixes of a normalized model name, most specific first. Each family uses one set of
/// FIM tokens whatever its size, quantization or instruct tuning.
const FAMILIES: [(&str, EditPredictionPromptFormat); 10] = [
    ("qwen2.5-coder", EditPredictionPromptFormat::Qwen),
    ("qwen3-coder", EditPredictionPromptFormat::Qwen),
    ("qwen-coder", EditPredictionPromptFormat::Qwen),
    ("deepseek-coder", EditPredictionPromptFormat::DeepseekCoder),
    ("starcoder", EditPredictionPromptFormat::StarCoder),
    ("codellama", EditPredictionPromptFormat::CodeLlama),
    ("code-llama", EditPredictionPromptFormat::CodeLlama),
    ("codegemma", EditPredictionPromptFormat::CodeGemma),
    ("codestral", EditPredictionPromptFormat::Codestral),
    ("glm-4", EditPredictionPromptFormat::Glm),
];

/// The format of `model` by family: the file name without a directory, a `.gguf` extension or an
/// Ollama tag, in lower case, matched by prefix. `None` for an unknown family.
pub fn infer_by_family(model: &str) -> Option<EditPredictionPromptFormat> {
    let name = model
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(model)
        .split(':')
        .next()
        .unwrap_or(model)
        .to_ascii_lowercase();
    let name = name.strip_suffix(".gguf").unwrap_or(&name);
    FAMILIES
        .iter()
        .find(|(prefix, _)| name.starts_with(prefix))
        .map(|(_, format)| *format)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fim::infer_prompt_format;

    #[test]
    fn llama_cpp_and_ollama_names_find_their_family() {
        for (model, format) in [
            (
                "qwen2.5-coder-1.5b-instruct-q8_0.gguf",
                EditPredictionPromptFormat::Qwen,
            ),
            (
                r"C:\models\Qwen2.5-Coder-7B-Q4_K_M.gguf",
                EditPredictionPromptFormat::Qwen,
            ),
            ("qwen2.5-coder:7b", EditPredictionPromptFormat::Qwen),
            (
                "deepseek-coder-6.7b-base.Q5_K_M.gguf",
                EditPredictionPromptFormat::DeepseekCoder,
            ),
            ("starcoder2-3b", EditPredictionPromptFormat::StarCoder),
            ("codellama-7b-code", EditPredictionPromptFormat::CodeLlama),
            ("codegemma-2b", EditPredictionPromptFormat::CodeGemma),
            ("Codestral-22B-v0.1", EditPredictionPromptFormat::Codestral),
            ("glm-4-9b", EditPredictionPromptFormat::Glm),
        ] {
            assert_eq!(infer_prompt_format(model), Some(format), "{model}");
        }
    }

    #[test]
    fn exact_names_keep_upstream_results_and_unknown_names_stay_unknown() {
        assert_eq!(
            infer_prompt_format("qwen"),
            Some(EditPredictionPromptFormat::Qwen)
        );
        assert_eq!(
            infer_prompt_format("sweep-next-edit-1.5b"),
            Some(EditPredictionPromptFormat::Sweep)
        );
        assert_eq!(infer_prompt_format("llama-3.1-8b-instruct"), None);
        assert_eq!(infer_prompt_format(""), None);
    }
}
