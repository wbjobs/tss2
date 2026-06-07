#!/usr/bin/env python3
"""
Test client for WASI Code Executor Service
"""
import requests
import json
import time

BASE_URL = "http://localhost:8080"


def test_health():
    """Test health endpoint"""
    print("\n=== Testing /health ===")
    try:
        response = requests.get(f"{BASE_URL}/health")
        print(f"Status: {response.status_code}")
        print(f"Response: {json.dumps(response.json(), indent=2)}")
        return response.status_code == 200
    except Exception as e:
        print(f"Error: {e}")
        return False


def test_functions():
    """Test functions endpoint"""
    print("\n=== Testing /functions ===")
    try:
        response = requests.get(f"{BASE_URL}/functions")
        print(f"Status: {response.status_code}")
        data = response.json()
        print(f"Available host functions ({len(data['functions'])}):")
        for func in data['functions']:
            print(f"  - {func}")
        return response.status_code == 200
    except Exception as e:
        print(f"Error: {e}")
        return False


def test_execute_python():
    """Test Python code execution"""
    print("\n=== Testing Python Execution ===")
    code = """
print('Hello from Python in WASI!')

# Test cross-language function calls
result_add = add(5, 3)
print(f'add(5, 3) = {result_add}')

result_mul = multiply(4, 7)
print(f'multiply(4, 7) = {result_mul}')

result_fib = fibonacci(10)
print(f'fibonacci(10) = {result_fib}')

result_prime = is_prime(17)
print(f'is_prime(17) = {result_prime}')

result_prime2 = is_prime(18)
print(f'is_prime(18) = {result_prime2}')

result_rand = random_int(1, 100)
print(f'random_int(1, 100) = {result_rand}')
"""
    payload = {
        "language": "python",
        "code": code.strip(),
        "timeout_ms": 5000
    }
    try:
        start = time.time()
        response = requests.post(f"{BASE_URL}/execute", json=payload)
        elapsed = (time.time() - start) * 1000
        print(f"Status: {response.status_code}")
        print(f"Request time: {elapsed:.2f}ms")
        data = response.json()
        print(f"Execution ID: {data['execution_id']}")
        print(f"Success: {data['success']}")
        print(f"Execution time: {data['execution_time_ms']}ms")
        if data['stdout']:
            print(f"Stdout:\n{data['stdout']}")
        if data['stderr']:
            print(f"Stderr:\n{data['stderr']}")
        if data.get('error'):
            print(f"Error: {data['error']}")
        return response.status_code == 200 and data['success']
    except Exception as e:
        print(f"Error: {e}")
        return False


def test_execute_javascript():
    """Test JavaScript code execution"""
    print("\n=== Testing JavaScript Execution ===")
    code = """
console.log('Hello from JavaScript in WASI!');

const addResult = add(10, 20);
console.log('add(10, 20) =', addResult);

const fibResult = fibonacci(15);
console.log('fibonacci(15) =', fibResult);

const primeResult = is_prime(23);
console.log('is_prime(23) =', primeResult);
"""
    payload = {
        "language": "javascript",
        "code": code.strip(),
        "timeout_ms": 5000
    }
    try:
        start = time.time()
        response = requests.post(f"{BASE_URL}/execute", json=payload)
        elapsed = (time.time() - start) * 1000
        print(f"Status: {response.status_code}")
        print(f"Request time: {elapsed:.2f}ms")
        data = response.json()
        print(f"Execution ID: {data['execution_id']}")
        print(f"Success: {data['success']}")
        print(f"Execution time: {data['execution_time_ms']}ms")
        if data['stdout']:
            print(f"Stdout:\n{data['stdout']}")
        if data['stderr']:
            print(f"Stderr:\n{data['stderr']}")
        return response.status_code == 200 and data['success']
    except Exception as e:
        print(f"Error: {e}")
        return False


def test_execute_ruby():
    """Test Ruby code execution"""
    print("\n=== Testing Ruby Execution ===")
    code = """
puts 'Hello from Ruby in WASI!'

result_add = add(15, 25)
puts "add(15, 25) = #{result_add}"

result_mul = multiply(6, 8)
puts "multiply(6, 8) = #{result_mul}"

result_fib = fibonacci(8)
puts "fibonacci(8) = #{result_fib}"

result_prime = is_prime(29)
puts "is_prime(29) = #{result_prime}"
"""
    payload = {
        "language": "ruby",
        "code": code.strip(),
        "timeout_ms": 5000
    }
    try:
        start = time.time()
        response = requests.post(f"{BASE_URL}/execute", json=payload)
        elapsed = (time.time() - start) * 1000
        print(f"Status: {response.status_code}")
        print(f"Request time: {elapsed:.2f}ms")
        data = response.json()
        print(f"Execution ID: {data['execution_id']}")
        print(f"Success: {data['success']}")
        print(f"Execution time: {data['execution_time_ms']}ms")
        if data['stdout']:
            print(f"Stdout:\n{data['stdout']}")
        if data['stderr']:
            print(f"Stderr:\n{data['stderr']}")
        return response.status_code == 200 and data['success']
    except Exception as e:
        print(f"Error: {e}")
        return False


def test_stats():
    """Test stats endpoint"""
    print("\n=== Testing /stats ===")
    try:
        response = requests.get(f"{BASE_URL}/stats")
        print(f"Status: {response.status_code}")
        data = response.json()
        print(json.dumps(data, indent=2))
        return response.status_code == 200
    except Exception as e:
        print(f"Error: {e}")
        return False


def test_invalid_code():
    """Test error handling for empty code"""
    print("\n=== Testing Error Handling (Empty Code) ===")
    payload = {
        "language": "python",
        "code": "",
        "timeout_ms": 5000
    }
    try:
        response = requests.post(f"{BASE_URL}/execute", json=payload)
        print(f"Status: {response.status_code}")
        data = response.json()
        print(f"Response: {json.dumps(data, indent=2)}")
        return response.status_code == 400
    except Exception as e:
        print(f"Error: {e}")
        return False


def test_memory_boundary_checks():
    """Test memory boundary checks and string length limits"""
    print("\n=== Testing Memory Boundary Checks ===")

    code = f"""
# Test that string length constants are available
print("MAX_STRING_LENGTH =", MAX_STRING_LENGTH)
print("CHUNK_SIZE =", CHUNK_SIZE)

# Test get_max_string_length
max_len = get_max_string_length()
print("get_max_string_length() =", max_len)

# Test get_chunk_size
chunk = get_chunk_size()
print("get_chunk_size() =", chunk)

# Test check_memory_bounds with valid range
valid = check_memory_bounds(0, 1000)
print("check_memory_bounds(0, 1000) =", valid)

# Test check_memory_bounds with negative length
invalid = check_memory_bounds(0, -1)
print("check_memory_bounds(0, -1) =", invalid)

# Test string_length with valid length
len_ok = string_length(0, 1000)
print("string_length(0, 1000) =", len_ok)

# Test string_compare
cmp = string_compare(0, 5, 0, 10)
print("string_compare(0,5,0,10) =", cmp)
"""
    payload = {
        "language": "python",
        "code": code.strip(),
        "timeout_ms": 5000
    }
    try:
        response = requests.post(f"{BASE_URL}/execute", json=payload)
        print(f"Status: {response.status_code}")
        data = response.json()
        print(f"Success: {data['success']}")
        print(f"Stdout:\n{data['stdout']}")

        checks = [
            "MAX_STRING_LENGTH = 65536" in data["stdout"],
            "CHUNK_SIZE = 32768" in data["stdout"],
            "get_max_string_length() = 65536" in data["stdout"],
            "get_chunk_size() = 32768" in data["stdout"],
            "check_memory_bounds(0, 1000) = 1" in data["stdout"],
            "check_memory_bounds(0, -1) = 0" in data["stdout"],
            "string_length(0, 1000) = 1000" in data["stdout"],
        ]

        all_passed = all(checks)
        print(f"All boundary checks passed: {all_passed}")
        return response.status_code == 200 and data['success'] and all_passed
    except Exception as e:
        print(f"Error: {e}")
        return False


def test_execution_time_positive():
    """Test that execution time is always positive in stats"""
    print("\n=== Testing Execution Time is Always Positive ===")

    print("Executing several code samples...")
    for i in range(3):
        payload = {
            "language": "python",
            "code": f"print('Test {i}')\nprint('fib(10) =', fibonacci(10))",
            "timeout_ms": 5000
        }
        try:
            response = requests.post(f"{BASE_URL}/execute", json=payload)
            if response.status_code == 200:
                data = response.json()
                print(f"  Execution {i}: time = {data['execution_time_ms']}ms, positive = {data['execution_time_ms'] >= 0}")
        except Exception as e:
            print(f"  Error in execution {i}: {e}")

    print("Fetching stats...")
    try:
        response = requests.get(f"{BASE_URL}/stats")
        if response.status_code == 200:
            data = response.json()
            print(f"Average execution time: {data['average_execution_time_ms']}ms")
            print(f"Is positive: {data['average_execution_time_ms'] >= 0}")

            all_times_positive = True
            for lang_stat in data['by_language']:
                print(f"  {lang_stat['language']}: avg_time = {lang_stat['average_time_ms']}ms, positive = {lang_stat['average_time_ms'] >= 0}")
                if lang_stat['average_time_ms'] < 0:
                    all_times_positive = False

            for exec_record in data['recent_executions']:
                print(f"  Recent {exec_record['id'][:8]}: time = {exec_record['execution_time_ms']}ms, positive = {exec_record['execution_time_ms'] >= 0}")
                if exec_record['execution_time_ms'] < 0:
                    all_times_positive = False

            return (
                response.status_code == 200
                and data['average_execution_time_ms'] >= 0
                and all_times_positive
            )
    except Exception as e:
        print(f"Error: {e}")
        return False

    return False


def test_large_string_simulation():
    """Test large string handling (>64KB simulation)"""
    print("\n=== Testing Large String Handling Simulation ===")

    large_data = "x" * 1000
    code = f"""
# Test string operations with larger data
data = "{large_data}"
print("Data length:", len(data))
print("Fib(20) =", fibonacci(20))
print("Multiply(100, 200) =", multiply(100, 200))
print("Is Prime(997) =", is_prime(997))
"""

    payload = {
        "language": "javascript",
        "code": code.strip(),
        "timeout_ms": 5000
    }
    try:
        response = requests.post(f"{BASE_URL}/execute", json=payload)
        print(f"Status: {response.status_code}")
        data = response.json()
        print(f"Success: {data['success']}")
        print(f"Execution time: {data['execution_time_ms']}ms, positive = {data['execution_time_ms'] >= 0}")
        if data['stdout']:
            print(f"Stdout preview: {data['stdout'][:200]}...")

        return (
            response.status_code == 200
            and data['success']
            and data['execution_time_ms'] >= 0
        )
    except Exception as e:
        print(f"Error: {e}")
        return False


def main():
    print("=" * 60)
    print("WASI Code Executor Service - Integration Tests")
    print("=" * 60)

    results = []

    results.append(("Health Check", test_health()))
    results.append(("Functions List", test_functions()))
    results.append(("Python Execution", test_execute_python()))
    results.append(("JavaScript Execution", test_execute_javascript()))
    results.append(("Ruby Execution", test_execute_ruby()))
    results.append(("Stats Endpoint", test_stats()))
    results.append(("Invalid Code Handling", test_invalid_code()))
    results.append(("Memory Boundary Checks", test_memory_boundary_checks()))
    results.append(("Execution Time Positive", test_execution_time_positive()))
    results.append(("Large String Handling", test_large_string_simulation()))

    print("\n" + "=" * 60)
    print("Test Results Summary")
    print("=" * 60)
    passed = 0
    for name, result in results:
        status = "PASS" if result else "FAIL"
        print(f"  {name}: {status}")
        if result:
            passed += 1

    print(f"\nTotal: {passed}/{len(results)} tests passed")
    print("=" * 60)

    return passed == len(results)


if __name__ == "__main__":
    import sys
    success = main()
    sys.exit(0 if success else 1)
