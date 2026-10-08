// Exercises the exact parser used by the worker without loading any weights.
#define main worker_main
#define wmain worker_wmain
#include "main.cpp"
#undef main
#undef wmain
#include <cassert>
#include <sstream>
#include <functional>

static void rejects(const std::function<void()> &operation) {
  bool rejected = false;
  try { operation(); } catch (...) { rejected = true; }
  if (!rejected) throw std::runtime_error("expected rejection");
}
int main() {
  if (!pcm("").empty() || pcm("AAD/fwCA") != std::vector<float>{0, 32767 / 32768.0f, -1}) return 1;
  for (const auto &value : {"A", "AA==", "AB==", "AAB=", "====", "AA=A", "AA==AAAA", "AA?="})
    rejects([&] { pcm(value); });
  rejects([] { pcm(std::string(400000, 'A')); });
  if (integer(Json{{"id", uint64_t{7}}}, "id") != 7) return 1;
  rejects([] { integer(Json{{"id", -1}}, "id"); });
  rejects([] { integer(Json{{"id", 1.5}}, "id"); });
  rejects([] { integer(Json{{"id", uint64_t{9007199254740992ULL}}}, "id"); });
  if (language_code("English") != "en" || language_code("<|ja|>") != "ja" || !language_code("unknown").empty()) return 1;
  std::istringstream input("{\"type\":\"ping\"}\n");
  auto *previous = std::cin.rdbuf(input.rdbuf());
  std::string line;
  if (!read_line(line) || line != "{\"type\":\"ping\"}" || read_line(line)) return 1;
  std::istringstream oversized(std::string(kLine + 1, 'x'));
  std::cin.clear(); std::cin.rdbuf(oversized.rdbuf());
  rejects([&] { read_line(line); });
  std::istringstream truncated("{}");
  std::cin.clear(); std::cin.rdbuf(truncated.rdbuf());
  rejects([&] { read_line(line); });
  std::cin.clear(); std::cin.rdbuf(previous);
  return 0;
}
