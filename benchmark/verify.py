import tiktoken
import real_llm_bench as r

e = tiktoken.get_encoding("cl100k_base")

print("=== Layer 1: token arithmetic is reproducible (recount from the file's strings) ===")
print("A1_FACTS tokens         =", len(e.encode(r.A1_FACTS)))
base_writer = r.I_WRITE + r.DOC + "\n\n" + r.A1_FACTS + "\n\n" + r.A2_ANALYSIS
toap_writer = r.I_WRITE + r.A2_ANALYSIS
print("baseline writer prompt  =", len(e.encode(base_writer)))
print("toap writer prompt      =", len(e.encode(toap_writer)))

print("\n=== Layer 3: the 'LLM outputs' are author-written string literals, not captured runs ===")
print("type(A2_ANALYSIS)       =", type(r.A2_ANALYSIS).__name__)
print("A2_ANALYSIS char length =", len(r.A2_ANALYSIS))
print("there is no usage metadata, no request id, no logged transcript, no model name attached")

print("\n=== the completion-token 'parity' is forced by construction ===")
print("baseline and toap reuse the SAME output string A3_RECOMMENDATION ->",
      "identical by definition, not an empirical finding")
