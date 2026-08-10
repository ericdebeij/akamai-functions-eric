#!/bin/bash
# Smoke tests against `spin up` (default port 3000).
PORT=3001
spin build
spin up --listen 127.0.0.1:$PORT >test.log 2>&1 &
SPIN_PID=$!
sleep 2

# Download the fixture that is already in the bucket
echo "Downloading text1.txt from S3 bucket..."
curl -s -X GET http://127.0.0.1:$PORT/storage/text1.txt -D - | head -n 1

echo "Upload a file to S3 bucket..."
# Upload a file (S3 object upload is PUT, not POST)
curl -s -X PUT http://127.0.0.1:$PORT/storage/test.txt \
  -H 'content-type: text/plain' --data-binary "Hello from Spin Wasm!" -D - | head -n 1

echo "Downloading the uploaded file..."
# Download the file
curl -s -X GET http://127.0.0.1:$PORT/storage/test.txt -D - | head -n 1

echo "Delete the file should return 204 No Content"
curl -s -X DELETE http://127.0.0.1:$PORT/storage/test.txt -D - | head -n 1

echo "Non existing file should return 404 Not Found"
curl -s -X GET http://127.0.0.1:$PORT/storage/sub/dir/text1.txt -D - | head -n 1


# Anything other than GET/PUT/DELETE gets a 405
echo "POST should return 405 Method Not Allowed"
curl -i -s -X POST http://127.0.0.1:$PORT/storage/test.txt -D - | head -n 1

kill $SPIN_PID