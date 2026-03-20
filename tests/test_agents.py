import os
import sys
import tempfile
from importlib.machinery import SourceFileLoader
from importlib.util import spec_from_loader

# Load the standalone 'agents' script as a module
_agents_path = os.path.join(os.path.dirname(os.path.dirname(__file__)), "agents")
loader = SourceFileLoader("agents_module", _agents_path)
spec = spec_from_loader("agents_module", loader)
_agents_module = type(sys)("agents_module")
spec.loader.exec_module(_agents_module)
sys.modules["agents_module"] = _agents_module

TARGETS = _agents_module.TARGETS
doc = _agents_module.doc
main = _agents_module.main


def test_doc_copies_to_all_targets():
    with tempfile.TemporaryDirectory() as tmpdir:
        # Setup source file
        src_dir = os.path.join(tmpdir, ".agents")
        os.makedirs(src_dir)
        content = "# My Project\n\nUse pytest. Format with ruff.\n"
        with open(os.path.join(src_dir, "AGENT.md"), "w") as f:
            f.write(content)

        # Run
        main(["doc", "--root", tmpdir])

        # Verify every unique target was written with correct content
        seen_paths: set[str] = set()
        for cli, rel in TARGETS.items():
            dest = os.path.join(tmpdir, rel)
            if dest in seen_paths:
                continue
            seen_paths.add(dest)
            assert os.path.isfile(dest), f"{rel} was not created"
            assert open(dest).read() == content, f"{rel} has wrong content"

        # Verify exact set of files created
        expected = {os.path.normpath(r) for r in TARGETS.values()}
        assert expected == {
            "CLAUDE.md",
            "AGENTS.md",
            "GEMINI.md",
            os.path.normpath(".github/copilot-instructions.md"),
            "QWEN.md",
        }
