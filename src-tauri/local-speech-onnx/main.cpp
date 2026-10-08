// Private Mimi worker. No capture, network, Python or model repository code.
#include <sherpa-onnx/c-api/c-api.h>
#include <nlohmann/json.hpp>
#include <cstdint>
#include <cstdio>
#include <iostream>
#include <map>
#include <memory>
#include <stdexcept>
#include <string>
#include <vector>
#ifdef _WIN32
#define WIN32_LEAN_AND_MEAN
#define NOMINMAX
#include <windows.h>
#include <fcntl.h>
#include <io.h>
#endif
using Json = nlohmann::json;
constexpr size_t kLine = 65536, kSamples = 16000 * 8, kText = 32768;

static void emit(const Json &event) {
  const auto line = event.dump();
  if (line.size() > kLine) throw std::runtime_error("output_limit");
  std::cout << line << '\n' << std::flush;
}
static bool read_line(std::string &line) {
  line.clear();
  char c;
  while (std::cin.get(c)) {
    if (c == '\n') return true;
    if (line.size() == kLine) throw std::runtime_error("input_limit");
    line.push_back(c);
  }
  if (!line.empty()) throw std::runtime_error("truncated_input");
  return false;
}
static uint64_t integer(const Json &event, const char *key) {
  const auto &value = event.at(key);
  if (!value.is_number_unsigned()) throw std::runtime_error("invalid_identity");
  auto result = value.get<uint64_t>();
  if (result > 9007199254740991ULL) throw std::runtime_error("identity_limit");
  return result;
}
static std::vector<float> pcm(const std::string &encoded) {
  // Strict padded base64; reject partial PCM16 rather than silently truncating.
  const std::string alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
  if (encoded.size() % 4 || encoded.size() > (kSamples * 2 + 2) / 3 * 4)
    throw std::runtime_error("invalid_audio");
  std::vector<uint8_t> bytes;
  for (size_t i = 0; i < encoded.size(); i += 4) {
    uint32_t bits = 0;
    int padding = 0;
    for (size_t j = 0; j < 4; ++j) {
      auto c = encoded[i + j];
      if (c == '=') {
        if (j < 2 || i + 4 != encoded.size()) throw std::runtime_error("invalid_audio");
        ++padding; bits <<= 6;
      } else {
        auto pos = alphabet.find(c);
        if (pos == std::string::npos || padding) throw std::runtime_error("invalid_audio");
        bits = (bits << 6) | static_cast<uint32_t>(pos);
      }
    }
    if ((padding == 1 && (bits & 0xff)) || (padding == 2 && (bits & 0xffff)))
      throw std::runtime_error("invalid_audio");
    bytes.push_back(static_cast<uint8_t>(bits >> 16));
    if (padding < 2) bytes.push_back(static_cast<uint8_t>(bits >> 8));
    if (!padding) bytes.push_back(static_cast<uint8_t>(bits));
  }
  if (bytes.size() % 2) throw std::runtime_error("invalid_audio");
  std::vector<float> samples;
  samples.reserve(bytes.size() / 2);
  for (size_t i = 0; i < bytes.size(); i += 2) {
    auto value = static_cast<int16_t>(static_cast<uint16_t>(bytes[i]) |
                                      (static_cast<uint16_t>(bytes[i + 1]) << 8));
    samples.push_back(value / 32768.0f);
  }
  return samples;
}

static const std::map<std::string, std::string> &language_codes() {
  static const std::map<std::string, std::string> codes = {
    {"Chinese", "zh"}, {"English", "en"}, {"Cantonese", "yue"}, {"Japanese", "ja"},
    {"Korean", "ko"}, {"Arabic", "ar"}, {"German", "de"}, {"French", "fr"},
    {"Spanish", "es"}, {"Portuguese", "pt"}, {"Indonesian", "id"}, {"Italian", "it"},
    {"Russian", "ru"}, {"Thai", "th"}, {"Vietnamese", "vi"}, {"Turkish", "tr"},
    {"Hindi", "hi"}, {"Malay", "ms"}, {"Dutch", "nl"}, {"Swedish", "sv"},
    {"Danish", "da"}, {"Finnish", "fi"}, {"Polish", "pl"}, {"Czech", "cs"},
    {"Filipino", "tl"}, {"Persian", "fa"}, {"Greek", "el"}, {"Hungarian", "hu"},
    {"Macedonian", "mk"}, {"Romanian", "ro"}
  };
  return codes;
}
static std::string language_code(const char *raw) {
  if (!raw) return "";
  size_t length = 0;
  while (length <= 32 && raw[length]) ++length;
  if (length > 32) return "";
  const std::string label(raw, length);
  for (const auto &entry : language_codes()) {
    if (label == entry.first || label == entry.second || label == "<|" + entry.second + "|>")
      return entry.second;
  }
  return "";
}
static std::string language_label(const std::string &code) {
  for (const auto &entry : language_codes()) {
    if (entry.second == code) return entry.first;
  }
  throw std::runtime_error("language");
}

// Model lifetime spans all turns; each stream and result is released immediately.
static int run(int argc, char **argv) {
  if (argc != 4) throw std::runtime_error("arguments");
  const std::string kind = argv[1], root = argv[2], language = argv[3];
  const auto hint = language == "auto" ? "" : language_label(language);
  const auto path = [&](const char *file) { return root + "/" + file; };
  const auto sense = path("model.int8.onnx"), tokens = path("tokens.txt");
  const auto conv = path("conv_frontend.onnx"), encoder = path("encoder.int8.onnx");
  const auto decoder = path("decoder.int8.onnx"), tokenizer = path("tokenizer");
  SherpaOnnxOfflineRecognizerConfig config{};
  config.feat_config.sample_rate = 16000;
  config.feat_config.feature_dim = 80;
  config.model_config.num_threads = 4;
  config.model_config.provider = "cpu";
  config.decoding_method = "greedy_search";
  if (kind == "sense-voice") {
    if (language != "auto" && language != "zh" && language != "en" &&
        language != "ja" && language != "ko" && language != "yue")
      throw std::runtime_error("language");
    config.model_config.tokens = tokens.c_str();
    config.model_config.sense_voice.model = sense.c_str();
    config.model_config.sense_voice.language = language.c_str();
    config.model_config.sense_voice.use_itn = 1;
  } else if (kind == "qwen-onnx") {
    // Qwen's language hint is a stream option, not a recognizer config field.
    auto &qwen = config.model_config.qwen3_asr;
    qwen.conv_frontend = conv.c_str(); qwen.encoder = encoder.c_str();
    qwen.decoder = decoder.c_str(); qwen.tokenizer = tokenizer.c_str();
    qwen.max_total_len = 512; qwen.max_new_tokens = 256;
    qwen.temperature = 0; qwen.top_p = 1; qwen.seed = 0;
  } else throw std::runtime_error("engine");
  const auto *recognizer = SherpaOnnxCreateOfflineRecognizer(&config);
  if (!recognizer) throw std::runtime_error("load");
  std::unique_ptr<const SherpaOnnxOfflineRecognizer, decltype(&SherpaOnnxDestroyOfflineRecognizer)>
      owner(recognizer, SherpaOnnxDestroyOfflineRecognizer);
  emit({{"type", "ready"}});
  std::vector<float> samples;
  samples.reserve(kSamples);
  uint64_t id = 0, revision = 0;
  bool active = false;
  std::string line;
  while (read_line(line)) {
    const auto event = Json::parse(line);
    const auto type = event.at("type").get<std::string>();
    if (type == "start") {
      if (active) throw std::runtime_error("turn_overlap");
      id = integer(event, "id"); revision = integer(event, "revision");
      samples.clear(); active = true;
    } else if (type == "audio") {
      if (!active) throw std::runtime_error("turn_missing");
      auto chunk = pcm(event.at("audio").get<std::string>());
      if (chunk.size() > kSamples - samples.size()) throw std::runtime_error("audio_limit");
      samples.insert(samples.end(), chunk.begin(), chunk.end());
    } else if (type == "commit") {
      if (!active) throw std::runtime_error("turn_missing");
      std::string text;
      std::string detected;
      if (!samples.empty()) {
        std::unique_ptr<const SherpaOnnxOfflineStream, decltype(&SherpaOnnxDestroyOfflineStream)>
            stream(SherpaOnnxCreateOfflineStream(recognizer), SherpaOnnxDestroyOfflineStream);
        if (!stream) throw std::runtime_error("stream");
        if (kind == "qwen-onnx" && !hint.empty())
          SherpaOnnxOfflineStreamSetOption(stream.get(), "language", hint.c_str());
        SherpaOnnxAcceptWaveformOffline(stream.get(), 16000, samples.data(), static_cast<int32_t>(samples.size()));
        SherpaOnnxDecodeOfflineStream(recognizer, stream.get());
        std::unique_ptr<const SherpaOnnxOfflineRecognizerResult, decltype(&SherpaOnnxDestroyOfflineRecognizerResult)>
            result(SherpaOnnxGetOfflineStreamResult(stream.get()), SherpaOnnxDestroyOfflineRecognizerResult);
        if (!result || !result->text) throw std::runtime_error("result");
        // Never scan an unbounded native output string.
        size_t length = 0;
        while (length <= kText && result->text[length]) ++length;
        if (length > kText) throw std::runtime_error("text_limit");
        text.assign(result->text, length);
        detected = language_code(result->lang);
        // v1.13.8 strips Qwen's generated language prefix but does not return it
        // in result.lang. Preserve only an explicitly configured language.
        if (kind == "qwen-onnx" && !hint.empty()) detected = language;
      }
      samples.clear(); active = false;
      Json final = {{"type", "final"}, {"id", id}, {"revision", revision}, {"text", text}};
      if (!detected.empty()) final["language"] = detected;
      emit(final);
    } else if (type == "clear") {
      samples.clear(); active = false;
    } else if (type == "ping") {
      emit({{"type", "pong"}, {"id", integer(event, "id")}});
    } else if (type == "finish") {
      samples.clear(); emit({{"type", "finished"}}); return 0;
    } else throw std::runtime_error("command");
  }
  return 0;
}
static int worker_entry(int argc, char **argv) {
#ifdef _WIN32
  _setmode(_fileno(stdin), _O_BINARY); _setmode(_fileno(stdout), _O_BINARY);
#endif
  try { return run(argc, argv); }
  catch (...) { try { emit({{"type", "error"}}); } catch (...) {} return 1; }
}
#ifdef _WIN32
int wmain(int argc, wchar_t **wide_argv) {
  // Windows usernames and model paths must arrive at sherpa as UTF-8.
  std::vector<std::string> values;
  values.reserve(static_cast<size_t>(argc));
  for (int i = 0; i < argc; ++i) {
    const int length = WideCharToMultiByte(CP_UTF8, WC_ERR_INVALID_CHARS, wide_argv[i], -1, nullptr, 0, nullptr, nullptr);
    if (length <= 0) return 1;
    std::string value(static_cast<size_t>(length), '\0');
    if (WideCharToMultiByte(CP_UTF8, WC_ERR_INVALID_CHARS, wide_argv[i], -1, value.data(), length, nullptr, nullptr) != length) return 1;
    value.pop_back(); values.push_back(std::move(value));
  }
  std::vector<char *> arguments;
  for (auto &value : values) arguments.push_back(value.data());
  return worker_entry(argc, arguments.data());
}
#else
int main(int argc, char **argv) { return worker_entry(argc, argv); }
#endif
