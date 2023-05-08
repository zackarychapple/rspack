use std::borrow::Cow;
use std::hash::Hash;

use rspack_error::{internal_error, IntoTWithDiagnosticArray, Result, TWithDiagnosticArray};
use rspack_identifier::{Identifiable, Identifier};

use crate::{
  rspack_sources::{BoxSource, RawSource, Source, SourceExt},
  to_identifier, AstOrSource, BuildContext, BuildResult, ChunkInitFragments, CodeGenerationResult,
  Compilation, Context, ExternalType, GenerationResult, InitFragment, InitFragmentStage,
  LibIdentOptions, Module, ModuleType, RuntimeGlobals, SourceType,
};

static EXTERNAL_MODULE_JS_SOURCE_TYPES: &[SourceType] = &[SourceType::JavaScript];
static EXTERNAL_MODULE_CSS_SOURCE_TYPES: &[SourceType] = &[SourceType::Css];

#[derive(Debug)]
pub struct ExternalModule {
  id: Identifier,
  pub request: String,
  external_type: ExternalType,
  /// Request intended by user (without loaders from config)
  user_request: String,
}

impl ExternalModule {
  pub fn new(request: String, external_type: ExternalType, user_request: String) -> Self {
    Self {
      id: Identifier::from(format!("external {external_type} {request}")),
      request,
      external_type,
      user_request,
    }
  }

  fn get_source_for_commonjs(&self) -> String {
    format!("module.exports = require('{}')", self.request)
  }

  fn get_source_for_import(&self, compilation: &Compilation) -> String {
    format!(
      "module.exports = {}('{}')",
      compilation.options.output.import_function_name, self.request
    )
  }

  pub fn get_source(
    &self,
    compilation: &Compilation,
  ) -> (BoxSource, ChunkInitFragments, RuntimeGlobals) {
    let mut chunk_init_fragments: ChunkInitFragments = Default::default();
    let mut runtime_requirements: RuntimeGlobals = Default::default();
    let source = match self.external_type.as_str() {
      "this" => format!(
        "module.exports = (function() {{ return this['{}']; }}())",
        self.request
      ),
      "window" | "self" => format!(
        "module.exports = {}['{}']",
        self.external_type, self.request
      ),
      "global" => format!(
        "module.exports = {}['{}']",
        compilation.options.output.global_object, self.request
      ),
      "commonjs" | "commonjs2" | "commonjs-module" | "commonjs-static" => {
        self.get_source_for_commonjs()
      }
      "node-commonjs" => {
        if compilation.options.output.module {
          chunk_init_fragments
            .entry("external module node-commonjs".to_string())
            .or_insert(InitFragment::new(
              "import { createRequire as __WEBPACK_EXTERNAL_createRequire } from 'module';\n"
                .to_string(),
              InitFragmentStage::STAGE_HARMONY_IMPORTS,
              None,
            ));
          format!(
            "__WEBPACK_EXTERNAL_createRequire(import.meta.url)('{}')",
            self.request
          )
        } else {
          self.get_source_for_commonjs()
        }
      }
      "amd" | "amd-require" | "umd" | "umd2" | "system" | "jsonp" => {
        let id = compilation
          .module_graph
          .module_graph_module_by_identifier(&self.identifier())
          .map(|m| m.id(&compilation.chunk_graph))
          .unwrap_or_default();
        format!(
          "module.exports = __WEBPACK_EXTERNAL_MODULE_{}__",
          to_identifier(id)
        )
      }
      "import" => self.get_source_for_import(compilation),
      "var" | "promise" | "const" | "let" | "assign" => {
        format!("module.exports = {}", self.request)
      }
      "module" => {
        if compilation.options.output.module {
          let id = compilation
            .module_graph
            .module_graph_module_by_identifier(&self.identifier())
            .map(|m| m.id(&compilation.chunk_graph))
            .unwrap_or_default();
          let identifier = to_identifier(id);
          chunk_init_fragments
            .entry(format!("external module import {identifier}"))
            .or_insert(InitFragment::new(
              format!(
                "import * as __WEBPACK_EXTERNAL_MODULE_{identifier}__ from '{}';\n",
                self.request
              ),
              InitFragmentStage::STAGE_HARMONY_IMPORTS,
              None,
            ));
          runtime_requirements.add(RuntimeGlobals::DEFINE_PROPERTY_GETTERS);
          format!(
            r#"var x = y => {{ var x = {{}}; {}(x, y); return x; }}
            var y = x => () => x
            module.exports = __WEBPACK_EXTERNAL_MODULE_{identifier}__"#,
            RuntimeGlobals::DEFINE_PROPERTY_GETTERS,
          )
        } else {
          self.get_source_for_import(compilation)
        }
      }
      // TODO "script"
      _ => "".to_string(),
    };
    (
      RawSource::from(source).boxed(),
      chunk_init_fragments,
      runtime_requirements,
    )
  }
}

impl Identifiable for ExternalModule {
  fn identifier(&self) -> Identifier {
    self.id
  }
}

#[async_trait::async_trait]
impl Module for ExternalModule {
  fn module_type(&self) -> &ModuleType {
    &ModuleType::Js
  }

  fn source_types(&self) -> &[SourceType] {
    if self.external_type == "css-import" {
      EXTERNAL_MODULE_CSS_SOURCE_TYPES
    } else {
      EXTERNAL_MODULE_JS_SOURCE_TYPES
    }
  }

  fn original_source(&self) -> Option<&dyn Source> {
    None
  }

  fn readable_identifier(&self, _context: &Context) -> Cow<str> {
    Cow::Owned(format!("external {}", self.request))
  }

  fn size(&self, _source_type: &SourceType) -> f64 {
    // copied from webpack `ExternalModule`
    // roughly for url
    42.0
  }

  async fn build(
    &mut self,
    _build_context: BuildContext<'_>,
  ) -> Result<TWithDiagnosticArray<BuildResult>> {
    Ok(BuildResult::default().with_empty_diagnostic())
  }

  fn code_generation(&self, compilation: &Compilation) -> Result<CodeGenerationResult> {
    let mut cgr = CodeGenerationResult::default();
    match self.external_type.as_str() {
      "asset" => {
        cgr.add(
          SourceType::JavaScript,
          GenerationResult::from(AstOrSource::from(
            RawSource::from(format!(
              "module.exports = {};",
              serde_json::to_string(&self.request).map_err(|e| internal_error!(e.to_string()))?
            ))
            .boxed(),
          )),
        );
        cgr.data.insert("url".to_owned(), self.request.clone());
      }
      "css-import" => {
        cgr.add(
          SourceType::Css,
          GenerationResult::from(AstOrSource::from(
            RawSource::from(format!(
              "@import url({});",
              serde_json::to_string(&self.request).map_err(|e| internal_error!(e.to_string()))?
            ))
            .boxed(),
          )),
        );
      }
      _ => {
        let (source, chunk_init_fragments, runtime_requirements) = self.get_source(compilation);
        cgr.add(
          SourceType::JavaScript,
          GenerationResult::from(AstOrSource::from(source)),
        );
        cgr.chunk_init_fragments = chunk_init_fragments;
        cgr.runtime_requirements.add(runtime_requirements);
        cgr.set_hash();
      }
    };
    Ok(cgr)
  }

  fn lib_ident(&self, _options: LibIdentOptions) -> Option<Cow<str>> {
    Some(Cow::Borrowed(self.user_request.as_str()))
  }
}

impl Hash for ExternalModule {
  fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
    "__rspack_internal__ExternalModule".hash(state);
    self.identifier().hash(state);
  }
}

impl PartialEq for ExternalModule {
  fn eq(&self, other: &Self) -> bool {
    self.identifier() == other.identifier()
  }
}

impl Eq for ExternalModule {}
