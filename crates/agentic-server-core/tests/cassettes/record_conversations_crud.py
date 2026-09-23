#!/usr/bin/env python3
"""
Record Conversations API CRUD operations for cassette testing.

This script records the following scenarios:
1. basic-crud: Create conversation, add items, list/retrieve/delete items, delete conversation
2. pagination: Test pagination with order (asc/desc) and after cursor
3. stateful-context: Add items manually, then make Responses call to verify context
4. deletion-continuation: Delete item, verify continuation behavior
5. branching: Branch from responses before/after manual item additions
6. error-cases: Invalid requests, missing resources, validation errors

Each scenario is recorded against OpenAI and the gateway for comparison.
"""

import argparse
import json
import os
import sys
import time
from pathlib import Path
from typing import Any, Optional

import httpx
from yaml import dump as yaml_dump


class ConversationsRecorder:
    """Records Conversations API operations into cassette YAML."""

    def __init__(
        self,
        provider: str,
        base_url: str,
        model: str,
        api_key: Optional[str] = None,
    ):
        self.provider = provider
        self.base_url = base_url.rstrip("/")
        self.model = model
        self.api_key = api_key or os.getenv("OPENAI_API_KEY", "")
        self.turns: list[dict] = []
        self.turn_number = 0

    def _headers(self) -> dict:
        """Build request headers."""
        headers = {"Content-Type": "application/json"}
        if self.api_key:
            headers["Authorization"] = f"Bearer {self.api_key}"
        return headers

    def _record_turn(
        self,
        method: str,
        path: str,
        request_body: Optional[dict] = None,
        response_status: int = 200,
        response_body: Optional[dict] = None,
        query_params: Optional[dict] = None,
    ):
        """Record a single request/response turn."""
        self.turn_number += 1

        turn = {
            "filename": f"t{self.turn_number}",
            "request": {
                "method": method,
                "path": path,
                "headers": {"content-type": "application/json"},
                "query_params": query_params or {},
            },
            "response": {
                "status_code": response_status,
                "headers": {"content-type": "application/json"},
            },
        }

        if request_body is not None:
            turn["request"]["body"] = request_body

        if response_body is not None:
            turn["response"]["body"] = response_body

        self.turns.append(turn)

    def _make_request(
        self,
        method: str,
        path: str,
        body: Optional[dict] = None,
        query_params: Optional[dict] = None,
    ) -> tuple[int, dict]:
        """Make actual HTTP request and record it."""
        url = f"{self.base_url}{path}"
        if query_params:
            params_str = "&".join(f"{k}={v}" for k, v in query_params.items())
            url = f"{url}?{params_str}"

        with httpx.Client(timeout=60.0) as client:
            if method == "POST":
                resp = client.post(url, json=body, headers=self._headers())
            elif method == "GET":
                resp = client.get(url, headers=self._headers())
            elif method == "PATCH":
                resp = client.patch(url, json=body, headers=self._headers())
            elif method == "DELETE":
                resp = client.delete(url, headers=self._headers())
            else:
                raise ValueError(f"Unsupported method: {method}")

            resp_body = resp.json() if resp.content else {}
            self._record_turn(
                method, path, body, resp.status_code, resp_body, query_params
            )
            return resp.status_code, resp_body

    def record_basic_crud(self):
        """Record basic CRUD operations on conversations and items."""
        print("Recording basic CRUD scenario...")

        # 1. Create conversation
        print("  1. Creating conversation...")
        status, conv = self._make_request(
            "POST",
            "/v1/conversations",
            {"metadata": {"test": "basic-crud", "session": "1"}},
        )
        assert status == 200, f"Expected 200, got {status}"
        conv_id = conv["id"]

        # 2. Retrieve conversation
        print(f"  2. Retrieving conversation {conv_id}...")
        status, retrieved = self._make_request("GET", f"/v1/conversations/{conv_id}")
        assert status == 200
        assert retrieved["id"] == conv_id

        # 3. Update conversation metadata
        print("  3. Updating metadata...")
        status, updated = self._make_request(
            "PATCH",
            f"/v1/conversations/{conv_id}",
            {"metadata": {"test": "basic-crud", "session": "1", "updated": "true"}},
        )
        assert status == 200

        # 4. Create items
        print("  4. Creating items...")
        items_to_create = [
            {"type": "message", "role": "user", "content": "Hello, I'm testing the Conversations API"},
            {"type": "message", "role": "assistant", "content": "Hello! How can I help you test the API today?"},
            {"type": "message", "role": "user", "content": "Can you remember the word APPLE?"},
        ]
        status, create_resp = self._make_request(
            "POST",
            f"/v1/conversations/{conv_id}/items",
            {"items": items_to_create},
        )
        assert status == 200
        assert len(create_resp["data"]) == 3
        item_ids = [item["id"] for item in create_resp["data"]]

        # 5. List items
        print("  5. Listing items...")
        status, list_resp = self._make_request(
            "GET",
            f"/v1/conversations/{conv_id}/items",
            query_params={"limit": "10"},
        )
        assert status == 200
        assert len(list_resp["data"]) == 3

        # 6. Retrieve single item
        print(f"  6. Retrieving item {item_ids[0]}...")
        status, item = self._make_request(
            "GET",
            f"/v1/conversations/{conv_id}/items/{item_ids[0]}",
        )
        assert status == 200
        assert item["id"] == item_ids[0]

        # 7. Delete an item
        print(f"  7. Deleting item {item_ids[1]}...")
        status, delete_resp = self._make_request(
            "DELETE",
            f"/v1/conversations/{conv_id}/items/{item_ids[1]}",
        )
        assert status == 200
        assert delete_resp["deleted"] is True

        # 8. List items after deletion
        print("  8. Listing items after deletion...")
        status, list_after = self._make_request(
            "GET",
            f"/v1/conversations/{conv_id}/items",
        )
        assert status == 200
        assert len(list_after["data"]) == 2

        # 9. Delete conversation
        print(f"  9. Deleting conversation {conv_id}...")
        status, delete_conv = self._make_request(
            "DELETE",
            f"/v1/conversations/{conv_id}",
        )
        assert status == 200
        assert delete_conv["deleted"] is True

        print("✓ Basic CRUD scenario recorded")

    def record_pagination(self):
        """Record pagination scenarios with order and cursor."""
        print("Recording pagination scenario...")

        # Create conversation with many items
        print("  1. Creating conversation...")
        status, conv = self._make_request(
            "POST", "/v1/conversations", {"metadata": {"test": "pagination"}}
        )
        conv_id = conv["id"]

        # Create 10 items
        print("  2. Creating 10 items...")
        items = [
            {"type": "message", "role": "user" if i % 2 == 0 else "assistant", "content": f"Message {i}"}
            for i in range(10)
        ]
        status, create_resp = self._make_request(
            "POST", f"/v1/conversations/{conv_id}/items", {"items": items}
        )
        assert len(create_resp["data"]) == 10

        # Test descending order (default)
        print("  3. Listing with desc order (default)...")
        status, desc_page1 = self._make_request(
            "GET",
            f"/v1/conversations/{conv_id}/items",
            query_params={"limit": "5", "order": "desc"},
        )
        assert len(desc_page1["data"]) == 5
        assert desc_page1["has_more"] is True

        # Test pagination with cursor
        print("  4. Listing next page with cursor...")
        cursor_id = desc_page1["last_id"]
        status, desc_page2 = self._make_request(
            "GET",
            f"/v1/conversations/{conv_id}/items",
            query_params={"limit": "5", "order": "desc", "after": cursor_id},
        )
        assert len(desc_page2["data"]) == 5

        # Test ascending order
        print("  5. Listing with asc order...")
        status, asc_list = self._make_request(
            "GET",
            f"/v1/conversations/{conv_id}/items",
            query_params={"limit": "5", "order": "asc"},
        )
        assert len(asc_list["data"]) == 5

        # Verify ordering is different
        assert desc_page1["first_id"] != asc_list["first_id"], "Order should differ"

        print("✓ Pagination scenario recorded")

    def record_stateful_context(self):
        """Record manually added items affecting model context."""
        print("Recording stateful context scenario...")

        # Create conversation
        print("  1. Creating conversation...")
        status, conv = self._make_request(
            "POST", "/v1/conversations", {"metadata": {"test": "stateful"}}
        )
        conv_id = conv["id"]

        # Manually add context items
        print("  2. Adding manual context items...")
        context_items = [
            {"type": "message", "role": "user", "content": "Remember the word BANANA"},
            {"type": "message", "role": "assistant", "content": "I'll remember BANANA"},
        ]
        status, items_resp = self._make_request(
            "POST",
            f"/v1/conversations/{conv_id}/items",
            {"items": context_items},
        )

        # Make a Responses API call using the conversation
        print("  3. Making Responses call with conversation context...")
        status, response = self._make_request(
            "POST",
            "/v1/responses",
            {
                "model": self.model,
                "input": "What word did I ask you to remember?",
                "conversation": conv_id,
                "store": True,
                "stream": False,
            },
        )
        assert status == 200
        # Note: The actual response content depends on the model,
        # but the cassette captures the full request/response structure

        print("✓ Stateful context scenario recorded")

    def record_deletion_continuation(self):
        """Record item deletion affecting continuation."""
        print("Recording deletion continuation scenario...")

        # Create conversation with initial exchange
        print("  1. Creating conversation with Responses...")
        status, conv = self._make_request(
            "POST", "/v1/conversations", {}
        )
        conv_id = conv["id"]

        # First Responses call
        status, resp1 = self._make_request(
            "POST",
            "/v1/responses",
            {
                "model": self.model,
                "input": "Say: FIRST",
                "conversation": conv_id,
                "store": True,
                "stream": False,
            },
        )
        resp1_id = resp1["id"]

        # Second Responses call
        status, resp2 = self._make_request(
            "POST",
            "/v1/responses",
            {
                "model": self.model,
                "input": "Say: SECOND",
                "previous_response_id": resp1_id,
                "store": True,
                "stream": False,
            },
        )
        resp2_id = resp2["id"]

        # List items to see what was created
        status, items_before = self._make_request(
            "GET", f"/v1/conversations/{conv_id}/items"
        )
        item_count_before = len(items_before["data"])

        # Delete an item
        print(f"  2. Deleting an item...")
        item_to_delete = items_before["data"][0]["id"]
        status, del_resp = self._make_request(
            "DELETE",
            f"/v1/conversations/{conv_id}/items/{item_to_delete}",
        )

        # List items after deletion
        status, items_after = self._make_request(
            "GET", f"/v1/conversations/{conv_id}/items"
        )
        assert len(items_after["data"]) == item_count_before - 1

        # Try continuation with conversation_id
        print("  3. Continuing with conversation_id...")
        status, resp3 = self._make_request(
            "POST",
            "/v1/responses",
            {
                "model": self.model,
                "input": "Say: THIRD",
                "conversation": conv_id,
                "store": True,
                "stream": False,
            },
        )

        # Try continuation with previous_response_id (from before deletion)
        print("  4. Continuing with previous_response_id...")
        status, resp4 = self._make_request(
            "POST",
            "/v1/responses",
            {
                "model": self.model,
                "input": "Say: FOURTH",
                "previous_response_id": resp2_id,
                "store": True,
                "stream": False,
            },
        )

        print("✓ Deletion continuation scenario recorded")

    def record_branching(self):
        """Record branching before/after manual item additions."""
        print("Recording branching scenario...")

        # Create conversation
        status, conv = self._make_request("POST", "/v1/conversations", {})
        conv_id = conv["id"]

        # First response
        print("  1. First response...")
        status, resp1 = self._make_request(
            "POST",
            "/v1/responses",
            {
                "model": self.model,
                "input": "Count to 3",
                "conversation": conv_id,
                "store": True,
                "stream": False,
            },
        )
        resp1_id = resp1["id"]

        # Branch A: Continue before adding manual items
        print("  2. Branch A: Before manual items...")
        status, branch_a = self._make_request(
            "POST",
            "/v1/responses",
            {
                "model": self.model,
                "input": "Now count to 5",
                "previous_response_id": resp1_id,
                "store": True,
                "stream": False,
            },
        )

        # Add manual items
        print("  3. Adding manual context items...")
        status, manual = self._make_request(
            "POST",
            f"/v1/conversations/{conv_id}/items",
            {
                "items": [
                    {"type": "message", "role": "user", "content": "Remember: ORANGE"},
                    {"type": "message", "role": "assistant", "content": "Got it: ORANGE"},
                ]
            },
        )

        # Branch B: Continue after adding manual items
        print("  4. Branch B: After manual items...")
        status, branch_b = self._make_request(
            "POST",
            "/v1/responses",
            {
                "model": self.model,
                "input": "What word did I tell you to remember?",
                "previous_response_id": resp1_id,
                "store": True,
                "stream": False,
            },
        )

        print("✓ Branching scenario recorded")

    def record_error_cases(self):
        """Record error cases and validation failures."""
        print("Recording error cases...")

        # 1. Retrieve non-existent conversation
        print("  1. Retrieving non-existent conversation...")
        status, err1 = self._make_request(
            "GET",
            "/v1/conversations/conv_nonexistent",
        )
        # Expect 404

        # 2. Create items with invalid data
        print("  2. Creating items with invalid request...")
        # First create a valid conversation
        status, conv = self._make_request("POST", "/v1/conversations", {})
        conv_id = conv["id"]

        # Try to create items with empty array
        status, err2 = self._make_request(
            "POST",
            f"/v1/conversations/{conv_id}/items",
            {"items": []},
        )
        # Expect 400

        # 3. Invalid order parameter
        print("  3. Listing with invalid order parameter...")
        # Create some items first
        status, _ = self._make_request(
            "POST",
            f"/v1/conversations/{conv_id}/items",
            {"items": [{"type": "message", "role": "user", "content": "test"}]},
        )

        status, err3 = self._make_request(
            "GET",
            f"/v1/conversations/{conv_id}/items",
            query_params={"order": "invalid"},
        )
        # Expect 400

        # 4. Include parameter (unsupported)
        print("  4. Listing with unsupported include parameter...")
        status, err4 = self._make_request(
            "GET",
            f"/v1/conversations/{conv_id}/items",
            query_params={"include": "metadata"},
        )
        # Expect 400

        print("✓ Error cases scenario recorded")

    def save_cassette(self, output_path: Path):
        """Save recorded turns to YAML cassette."""
        cassette = {"turns": self.turns}

        output_path.parent.mkdir(parents=True, exist_ok=True)
        with open(output_path, "w") as f:
            yaml_dump(
                cassette,
                f,
                default_flow_style=False,
                allow_unicode=True,
                sort_keys=False,
            )

        print(f"\n✓ Saved cassette: {output_path}")
        print(f"  Recorded {len(self.turns)} turns")


def main():
    parser = argparse.ArgumentParser(
        description="Record Conversations API CRUD operations"
    )
    parser.add_argument(
        "--provider",
        choices=["openai", "gateway"],
        required=True,
        help="Provider to record against",
    )
    parser.add_argument("--model", default="gpt-4o", help="Model name")
    parser.add_argument(
        "--gateway-url",
        default="http://localhost:9000",
        help="Gateway URL (for gateway provider)",
    )
    parser.add_argument(
        "--scenario",
        required=True,
        choices=[
            "basic-crud",
            "pagination",
            "stateful-context",
            "deletion-continuation",
            "branching",
            "error-cases",
        ],
        help="Scenario to record",
    )
    parser.add_argument("--output", required=True, help="Output cassette file")

    args = parser.parse_args()

    # Determine base URL and API key
    if args.provider == "openai":
        base_url = "https://api.openai.com"
        api_key = os.getenv("OPENAI_API_KEY")
        if not api_key:
            print("ERROR: OPENAI_API_KEY environment variable required", file=sys.stderr)
            sys.exit(1)
    else:
        base_url = args.gateway_url
        api_key = None  # Gateway may not require auth in dev

    recorder = ConversationsRecorder(args.provider, base_url, args.model, api_key)

    # Record the selected scenario
    try:
        if args.scenario == "basic-crud":
            recorder.record_basic_crud()
        elif args.scenario == "pagination":
            recorder.record_pagination()
        elif args.scenario == "stateful-context":
            recorder.record_stateful_context()
        elif args.scenario == "deletion-continuation":
            recorder.record_deletion_continuation()
        elif args.scenario == "branching":
            recorder.record_branching()
        elif args.scenario == "error-cases":
            recorder.record_error_cases()

        recorder.save_cassette(Path(args.output))

    except Exception as e:
        print(f"\nERROR: Recording failed: {e}", file=sys.stderr)
        import traceback
        traceback.print_exc()
        sys.exit(1)


if __name__ == "__main__":
    main()
